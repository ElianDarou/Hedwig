# Prüfbericht

Stand: 22. September 2026 — Entwicklungsfassung 0.1.0

## In diesem Lauf bestanden

- `cargo test --workspace --no-fail-fast`: 7 Sync-End-to-End-Tests, 6
  Core-Tests, 2 Linux-Mount-Strukturtests, 7 Server-Unit-/Migrationstests und
  29 Server-API-Integrationstests; keine Fehler. Der hostabhängige echte
  FUSE-Test blieb in dieser Standardsuite wie vorgesehen ignoriert.
- Die API-Prüfung umfasst unter anderem Block-/Space-Isolation, Rollen,
  Gruppen, Auth/2FA/Recovery/Geräte, Quoten, Archivschutz, Freigaben,
  idempotente Uploads, Datei-/Ordnerkollisionen, atomare Ordner-Moves,
  Versions- und Ordner-Restore, Backup/Restore, Backup-Retention,
  Upload-Sperrliste, Nur-Lese-Spiegel, Cross-Space-Kopie und signierte
  Webhook-Zustellung.
- Der Sync-Test verwendet jeweils einen echten lokalen HTTP-Server und getrennte
  Clientwurzeln. Er prüft Upload/Download, Rename ohne Inhaltsupload,
  gleichzeitige Bearbeitung/Umbenennung, Konfliktkopien, `.cloudignore`,
  Löschwellenpause, selektiven Sync ohne Remote-Löschung sowie tatsächliche
  Block-Nutzlast bei großer Dateiänderung. Gemessen wurden 8.388.608 Byte beim
  Erstupload, 631.036 Byte nach einer 32-KiB-Änderung und 0 Byte beim Rename.
- `npm run build` in `web` und `desktop`: TypeScript und Produktionsbundles
  erfolgreich.
- `ANDROID_HOME=/home/elian/Android/Sdk ./gradlew :app:lintDebug
  :app:testDebugUnitTest :app:connectedDebugAndroidTest :app:assembleRelease`:
  Android-17/API-37-Build mit AGP
  9.4.0 und Gradle 9.7.1 erfolgreich; Lint meldet keine Probleme. Geprüft wurden
  außerdem Mindest-API 26, Ziel-API 37, App-Icon, Android-14-Medienauswahl,
  Signatur, Manifest und kontrollierte Nutzer-CA-Unterstützung für private
  Homelab-Zertifikate. Das native Debug-APK ist 22.203.335 Byte groß und hat
  SHA-256
  `896f0ab2a3cd89467fbc690be5e32d6c9e7e0eefbafbdbd06a56ed186257dc6b`.
- Auf einem hardwarebeschleunigten Android-16-/API-36-Emulator bestanden vier
  Instrumentationstests: isolierte Formularbedienung mit dauerhaft sichtbarem
  Fehler, echte HTTPS-Authentifizierung gegen Springfield mit 401-Gegenprobe
  und Space-Abruf, ein kompletter UI-Login mit Navigation durch Offline,
  Freigaben, Papierkorb und Einstellungen sowie ein bereinigter Datenfluss mit
  Mehrblock-Upload, idempotenter Wiederholung, bytegleichem Download, zweiter
  Version, Historie, passwortgeschützter Freigabe samt Widerruf, Papierkorb und
  Wiederherstellung. Die private CA war als Nutzer-CA installiert. Heller und
  dunkler Modus sowie 130 % Schriftgröße wurden anhand echter
  Emulator-Screenshots geprüft.
- Die Android-Release-APK wurde mit einem getrennt aufbewahrten RSA-3072-
  Betreiberschlüssel gebaut. `apksigner verify` bestätigt APK-Signatur v2 und
  genau einen Unterzeichner. Das Artefakt ist 15.407.942 Byte groß; SHA-256:
  `f2bf22a38f53c73e1c4c5eb5f95ff0ce45d88da9640d7319d1a257f0f73bfacc`.
  Der Zertifikat-Fingerabdruck ist
  `255ed02ad1ca108e241631e9bd4e78479ec27e3a2fa6f3929f7482c19b28f9a8`.
  Ein erzwungener zweiter Release-Build enthielt dieselben 142 ZIP-Einträge mit
  identischen Nutzdaten und Metadaten und wurde mit demselben Zertifikat gültig
  signiert. Der äußere APK-Hash wich wegen des neu erzeugten v2-Signaturblocks
  ab; die dokumentierte Prüfsumme bezeichnet deshalb bewusst das veröffentlichte
  Artefakt und nicht beliebige spätere Signierläufe.
- `docker build -f desktop/Containerfile.check .`: Linux-Tauri-Rust-Backend mit
  WebKitGTK, Credential Store, Sync-Agent und FUSE kompiliert.
- `desktop/Containerfile.bundle`: Debian-Paket `Hedwig_0.1.0_amd64.deb`
  gebaut; Paketmetadaten, ELF-Abhängigkeiten, Desktopdatei und 512-px-Icon
  geprüft. Das aktuelle Paket ist 9.073.356 Byte groß; SHA-256:
  `cdc8c548160ddf6d659c6df835cf7ff45c5749234b74c8039c13ea4f65778433`.
  Zusätzlich installierte `dpkg` das Paket unter `fakeroot` in einen isolierten
  Root als `install ok installed` und registrierte 11 Payload-Einträge; nach
  `dpkg -r` waren Paketdatensatz, Binary und Desktopdatei vollständig entfernt.
- `docker build -f crates/hedwig-files-windows/Containerfile.check .`: CFAPI-Code
  erfolgreich für `x86_64-pc-windows-gnu` kompiliert.
- `docker compose config --quiet`: Compose-Datei gültig.
- Das Release-Server-Image wurde gebaut und als unprivilegierte UID 10001 mit
  getrennten Docker-Daten-/Backup-Volumes gestartet. Setup, Web-Auslieferung,
  Upload-Commit, manueller Backup-Lauf, Neustart und vollständige
  Container-Neuerstellung mit denselben Volumes bestanden. Anschließend wurden
  Backupmanifest und SQLite geprüft und in einen leeren Pfad wiederhergestellt.
- Ein echter Voll-Datenträgerlauf verwendete ein auf 8 MiB begrenztes Docker-
  `tmpfs`: Nach einer bestätigten 4-KiB-Datei scheiterte eine 12-MiB-Ersatzfassung
  sichtbar mit HTTP 507. Der Server blieb erreichbar; Inhalt, Dateiliste und
  Versionsliste antworteten weiter mit HTTP 200. Es blieb genau eine Version,
  und deren Versions-ID sowie SHA-256
  `ad7facb2586fc6e966c004d7d1d16b024f5805ff7cb47c7a85dabd8b48892ca7`
  waren unverändert. Ein Regressionstest prüft zusätzlich, dass ein unvollständiger
  Block nicht als vorhanden gilt und bei erneutem Upload repariert wird.
- `cargo audit` 0.22.2 prüfte 291 Rust-Abhängigkeiten gegen 1.256 RustSec-
  Advisories ohne Fund. `npm audit --omit=dev` meldete sowohl für Web als auch
  Desktop keine bekannte Schwachstelle. Dies ersetzt keinen Penetrationstest.

## Ausgeführte Lastmessung

Der manuelle, im Quelltext reproduzierbare Test
`benchmark_many_small_and_large_files` lief mit 2.000 kleinen Dateien und einer
128-MiB-Datei über einen echten lokalen HTTP-Server. Ergebnis: 135.625.728 Byte
Upload-Nutzlast, 132,603 Sekunden Upload, 164,905 Sekunden Download und 41.716
KiB maximaler Resident Set des gemeinsamen Testprozesses. Gemessen wurde auf
Ubuntu 24.04.4 LTS, Linux 7.0.0-30, Intel Core i5-6300U (2 Kerne/4 Threads),
15 GiB RAM und ext4. Dies ist eine lokale Entwicklungs-Hardwaremessung, kein
allgemeines Geschwindigkeitsversprechen.

## Zusätzlich bereits tatsächlich ausgeführt

Der Inhalt genau dieses aktuellen Debian-Pakets wurde auf dem Ubuntu-Host in
einen isolierten Pfad entpackt und das enthaltene `/usr/bin/cloud-desktop` in
der realen Wayland-/Xwayland-Sitzung gestartet. Die deutsche Anmeldung, der
native GTK-Ordnerdialog und die Tauri-WebView renderten ohne leeres Fenster oder
Überlauf. Anmeldung an einem isolierten Container-Server, Ordnerprofil und
automatischer beidseitiger Sync funktionierten: eine lokale Änderung erschien
nach 0,8 Sekunden auf dem Server, eine Änderung eines zweiten Clients nach 0,8
Sekunden lokal. `.cloudignore` hielt eine `*.tmp`-Datei ausschließlich lokal.
Der aus derselben GUI eingerichtete FUSE-Mount war tatsächlich `ro` eingehängt,
lieferte Inhalte bei Bedarf und lehnte Schreibzugriffe ab. Bei getrenntem
Container-Netz lieferte eine noch nicht geladene Datei einen I/O-Fehler und 0
Ausgabebytes; nach Wiederverbinden war der vollständige Inhalt lesbar.

Dieser Lauf deckte außerdem auf, dass der frühere Desktop-Logout nur den
lokalen Credential-Store bereinigte. Der gemeinsame Client und die Desktop-App
wurden korrigiert und das Paket danach neu gebaut. Ein neuer API-Regressionstest
belegt den Token-Widerruf. Im wiederholten Paket-GUI-Lauf sank die aktive
Serversitzung des Desktop-Geräts nach „Abmelden“ von 1 auf 0, während der lokale
Account ebenfalls entfernt wurde.

In einem früheren Lauf wurde der ignorierte Test
`actual_mount_loads_on_demand_and_reports_network_failure` ausdrücklich mit
einem nutzbaren `/dev/fuse` gestartet. Er lud Inhalte erst beim Lesen, hielt
bereits geladene Daten nach Serverausfall lesbar und lieferte für eine nicht
geladene Datei einen I/O-Fehler statt einer leeren Ersatzdatei.

Der aktuelle Webstand wurde gegen das gebaute Server-Image im Browser geprüft:
Login, persistierter Ordner/Dateieintrag und Administration mit Backupstatus,
Integrität, Upload-Sperren, Nutzern, Gruppen und Spaces waren bedienbar. Es gab
keine Seitenfehler oder Framework-Overlays. Bei 1280 px und bei 375 px Breite
entsprachen `scrollWidth` und `clientWidth`; es entstand kein horizontaler
Überlauf. Dunkles Farbschema und Reduced Motion wurden dabei angefordert.

Zusätzlich läuft das Image real auf `springfield` hinter Caddy-HTTPS. 1.660
kuratierte Projektdateien mit 37.087.455 Byte wurden hoch- und wieder
heruntergeladen; alle SHA-256-Werte stimmten. Eine 128-MiB-Datei erreichte beim
Erstupload 4,654 MiB/s, eine reale 1-MiB-Änderung übertrug nur 1.360.786 Byte
und damit 98,99 % weniger Container-Nutzlast. Bei einem gezielten Netzabbruch
endete der Agent sichtbar mit Fehler; der Folgelauf setzte mit den bereits
bestätigten Blöcken fort und die fertige Datei war hashgleich. Migration 12
wurde nach einer verifizierten Sicherung auf den Bestandsdaten ausgeführt.

Die aktuelle Springfield-Sicherung mit 2.494 Einträgen und 310.845.529 Byte
wurde mit `hedwigctl` geprüft, in ein leeres Verzeichnis zurückgespielt und als
zweiter Server gestartet. Dateizahl, Bytezähler, Schema und ein exemplarischer
Dateihash stimmten; Testserver und Restore-Verzeichnis wurden danach entfernt.
Der externe Browserlauf prüfte Login, Projekte, Administration und Logout bei
1.440 px und 390 px. Ein zuerst gefundener mobiler Logout-/Navigationsfehler
wurde korrigiert und nach dem Redeployment ohne horizontalen Überlauf oder
JavaScript-/HTTP-Fehler erneut bestanden. Alle Messdetails stehen in
[`springfield-verification.md`](springfield-verification.md).

Die signierte Android-APK, das aktuelle Debian-Paket, die öffentliche Homelab-
CA und eine Prüfsummendatei werden über Springfields Caddy unter `/install/`
ausgeliefert. Der erneute Download aller Artefakte über HTTPS stimmte mit den
lokalen SHA-256-Werten überein. Antworten verwenden `no-store` und
`Content-Disposition: attachment`; Verzeichnisauflistung und unbekannte Namen
liefern 404.

## Noch nicht ausgeführt

- kein echter Windows-10/11-Lauf: CFAPI-Platzhalter, Explorer-Aktionen,
  Änderung/Upload, Offlinefehler, Neustart, Installer und Deinstallation sind
  auf Windows ungetestet;
- kein physisches Android-Gerät: Der API-36-Emulatorlauf deckt Login, Spaces,
  Mehrblock-Upload/-Download, Versionen, Freigaben, Papierkorb,
  Shares-/Offline-/Einstellungsnavigation und Darstellungsvarianten ab.
  Kamera-Backup, tatsächliche Offline-Dateiübergabe, DocumentsProvider,
  SAF-Ordnerzugriff, Netzunterbrechung und eingeschränkte Hintergrundausführung
  sind auf echter Hardware weiterhin ungeprüft;
- keine systemweite Installation/Deinstallation des Debian-Pakets mit `dpkg`
  auf diesem Host, weil keine nichtinteraktiven Root-Rechte verfügbar waren;
  der isolierte Paketmanagerlauf sowie der reale Paketinhalt und seine
  GUI-/Sync-/FUSE-Pfade wurden wie oben beschrieben ausgeführt;
- kein Langzeittest;
- kein externer Sicherheits-/Penetrationstest und kein Backup-Lauf auf einem
  tatsächlich getrennten NAS.

Diese offenen Zielsystem- und Betriebsprüfungen verhindern weiterhin die
Freigabe als Version 1, obwohl die automatisierte Suite vollständig grün ist.
