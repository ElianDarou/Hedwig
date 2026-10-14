# Architekturentscheidungen

Stand: 20. September 2026

## Monolithischer Server

Der Server wird als Rust-Anwendung mit einer transaktionalen SQLite-Datenbank
und einem lokalen, inhaltsadressierten Blockspeicher gebaut. SQLite passt zum
Ziel eines einzelnen Homelab-Servers und hält Installation und Restore
überschaubar. Alle Metadatenänderungen, Versionen und Change-Log-Einträge werden
in einer Datenbanktransaktion bestätigt. Dateiblöcke werden vor dieser
Transaktion vollständig geschrieben und geprüft; unreferenzierte Blöcke bleiben
für eine spätere, lease-bewusste Bereinigung unsichtbar.

Blockdateien werden in eine temporäre Datei geschrieben, explizit geflusht und
per `fsync` gesichert, bevor ein atomarer Rename sie sichtbar macht. Ein
vorhandener Block muss die erwartete Größe besitzen; unvollständige Reste werden
bei einem erneuten Upload ersetzt. Speichermangel beim Blockschreiben oder beim
SQLite-Commit liefert HTTP 507 und verschiebt den aktuellen Versionszeiger nicht.

## Dateiinhalte

Der gemeinsame Rust-Core zerlegt Daten mit FastCDC in variable Blöcke und
adressiert sie mit BLAKE3. Block-IDs sind nur Inhaltskennungen, niemals
Zugriffstokens. Jede Dateiversion referenziert ihre vollständige geordnete
Blockliste. Deduplizierungsschlüssel enthalten immer die Space-ID.

Die Parameter sind Teil des versionierten Inhaltsformats. Format 1 verwendet
64 KiB Mindestgröße, 256 KiB Zielgröße und 1 MiB Maximalgröße. Ein späteres
Format darf vorhandene Versionen nicht unlesbar machen.

## Identitäten und Synchronisation

Dateien besitzen UUIDv7-Identitäten. Pfad, Name und aktueller Versionszeiger
sind getrennt vom Inhalt. Jeder bestätigte Vorgang erhält eine monoton steigende
Change-Sequenz. Clients bestätigen ihren Cursor; Live-Nachrichten sind nur ein
Beschleuniger dieses Protokolls. Der Server stellt dafür ein
authentifiziertes, inhaltsfreies Long-Poll-Wecksignal bereit. Es verrät keine
Objektdaten und kann gefahrlos verloren gehen, weil Change-Log und periodischer
Vollabgleich maßgeblich bleiben.

Mutationen verwenden einen Idempotenzschlüssel pro Gerät. Das Ergebnis wird
gespeichert und bei Wiederholung zurückgegeben. Upload-Sitzungen referenzieren
erwartete Blöcke und verfallen; erst `commit` erzeugt eine sichtbare Version.

## Konflikte

Der Client sendet die Ausgangsversion. Stimmt sie nicht mehr mit der aktuellen
Version überein, wird kein Inhalt überschrieben. Der Upload wird als neue Datei
mit verständlichem Konfliktnamen bestätigt. Delete-vs-edit und Rename-Konflikte
werden nach demselben Grundsatz als wiederauffindbare Objekte erhalten.

## Sicherheit

Passwörter werden mit Argon2id gehasht. Geräte- und Sitzungstokens werden nur
gehasht gespeichert. Der Reverse-Proxy terminiert HTTPS; der Server vertraut
standardmäßig keinem weitergereichten Benutzer-Header. Jede Objektoperation
prüft Space-Mitgliedschaft und Rolle. Diagnoseausgaben enthalten keine Tokens
oder Dateiinhalte.

## Clients

Das Protokoll und der Sync-Zustandsautomat liegen im Rust-Core. Tauri bindet ihn
direkt ein. Android nutzt die versionierte HTTP-API; eine spätere kleine JNI-
Bindung ist nur für exakt identische Chunking-/Ignore-Semantik vorgesehen.
Plattformintegrationen bleiben native Adapter. Der Linux-Adapter stellt einen
separaten read-only FUSE-Baum mit geprüftem On-Demand-Cache bereit. Der
Windows-Adapter verwendet CFAPI für Sync-Root, Explorer-Platzhalter,
blockgeprüfte Hydration, Upload geänderter Platzhalter sowie sicheres
Pin/Dehydrate. Android verwendet eine native Compose-App, WorkManager,
MediaStore und einen read-only DocumentsProvider. Der Android-Uploader nutzt
derzeit kompatible 1-MiB-Blöcke; die identische FastCDC-Grenzziehung des
Rust-Cores ist dort noch nicht eingebunden.

## Sicherung, Export und Wartung

Ein Online-Snapshot entsteht mit SQLite `VACUUM INTO`, danach wird der
Blockbaum unter einem gemeinsamen Wartungs-Lock kopiert und vollständig in
einem BLAKE3-Manifest erfasst. Restore prüft Manifest und SQLite, bevor ein
frischer Datenpfad sichtbar wird. Der normale Verzeichnisexport liest jede
Version direkt aus der vollständigen Blockliste und hasht jeden Block erneut.

Retention arbeitet direkt auf vollständigen Versionen: alle unter 24 Stunden,
mindestens eine je Stunde bis sieben Tage, je Tag bis 90 Tage und je Woche bis
zwei Jahre. Aktuelle, angeheftete und archivgeschützte Fassungen bleiben. Offene
Upload-Leases sowie der Backup-/Export-Lock schützen benötigte Blöcke.

## Persistenter Sync-Agent

`hedwig-agent` besitzt pro gewähltem Ordner eine SQLite-Zustandsdatenbank
außerhalb des Sync-Baums und sperrt parallele Läufe. Downloads werden vollständig
blockweise gegen das autorisierte Servermanifest geprüft, in eine temporäre
Datei im Zielordner geschrieben, synchronisiert und erst danach eingesetzt.

Vor entfernten Löschungen wird die Zahl und der Anteil lokal fehlender,
tatsächlich eingeschlossener Dateien vollständig ermittelt. Überschreitet dies
eine Schwelle, werden diese Löschungen nicht begonnen und der Prozess meldet
Exit-Code 2. Nicht erreichbare Wurzeln beenden den Lauf vor jeder Mutation.
Symlinks werden nicht verfolgt. Konfliktdateien bleiben sichtbar; von einem
anderen Gerät bestätigte Löschungen werden zunächst in den privaten
Client-Zustandsbereich verschoben.

Der Desktop-Host überwacht eingerichtete lokale Ordner rekursiv und entprellt
Dateisystemereignisse. Zusätzlich wartet er auf das Server-Wecksignal. Unabhängig
davon erzwingt er regelmäßig einen vollständigen Abgleich, damit verlorene
Dateisystem- oder Netzereignisse keinen dauerhaften Unterschied verursachen.
