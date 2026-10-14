# Erfüllungsmatrix

Stand: 22. September 2026. Diese Matrix trennt Implementierung, automatisierte
Prüfung und echte Zielplattformprüfung. Ein Cross-Compile ersetzt keinen Lauf
auf dem Zielsystem.

| Bereich | Implementierung | Prüfung |
|---|---|---|
| Inhaltsblöcke | FastCDC-Format 1, BLAKE3, vollständige Manifeste und Deduplizierung je Space; atomar geflushte und synchronisierte Blockdateien | Rekonstruktion, Wiederverwendung bei großer Änderung sowie Erkennung/Reparatur eines unvollständigen Blocks automatisiert |
| Sync-Core | Beidseitig, Upload-only, Download-only, Resume durch fehlende Blöcke, Idempotenz, Konflikte, Rename ohne Reupload, Massenlöschungsschutz, selektiver Sync, Ignore, Streaming-Upload/-Download und Bandbreitenlimits | 7 Live-HTTP-End-to-End-Tests bestanden; 8-MiB-Änderung übertrug gemessen nur 631.036 Byte, Rename 0 Byte |
| Servermetadaten | SQLite-Transaktionen, feste Dateiidentität, Ordner, Versionen, Papierkorb, Change-Log, Quoten und Archive | 7 Unit-/Migrationstests und 29 API-Integrationstests bestanden |
| Nutzer und Rechte | Admins, Nutzer, Gerätewiderruf, Gruppen, persönliche/gemeinsame Spaces und alle fünf Space-Rollen | Rollen-, Gruppen-, Geräte- und Fremdzugriffstests bestanden |
| Authentifizierung | Argon2id, verschlüsselte TOTP-Secrets, Recovery-Codes, Sitzungen und Passwortwechsel | Login-, 2FA-, Rekey-, Recovery- und Widerrufstests bestanden |
| Freigaben | Lese-/Download- und Upload-Links, Passwort, Ablauf, Größenlimit, Widerruf und Editor-Schalter | Normaler und Gastzugriff bis zum Commit automatisiert geprüft |
| Web-App | Datei-/Ordnerverwaltung, Mehrfachauswahl, Drag-and-drop, Suche, Vorschau, ZIP, Versionen mit Textdiff, Papierkorb, Ordner-Zeitreise, Shares, Space-/Gruppen-/Adminverwaltung | TypeScript-/Produktionsbuild und echter Springfield-Browserlauf bei 1.440 px sowie 390 px grün; mobile Navigation und serverseitiger Logout geprüft |
| Versionen und Papierkorb | Restore als neue Version, Pinning, gestufte Retention, 30 Tage Papierkorb und Ordner-Restore mit Vorschau | Retention, Einzel- und Ordner-Restore automatisiert geprüft |
| Backup/Restore/Export | konsistenter Snapshot, Hashmanifest, Prüfung, Zeitplan, Aufbewahrung, Admin-Status, Restore auf leeren Server sowie normaler Export | Manipulationserkennung, Retention und Wiederverbindung per Download automatisiert |
| Nur-Lese-Spiegel | optionaler atomar aktualisierter Baum, schreibgeschützte Rechte, Stand und Zusatzverbrauch | API-Integrationstest auf Linux bestanden |
| Administration | Quoten, Archive, Integrität, Diagnose, Backup, Spiegel, serverweite Upload-Sperrmuster | Adminrechte und zentrale Upload-Sperren für Geräte/Gäste geprüft |
| Linux Desktop | Tauri-App, klassischer Sync, Tray/Autostart, Dateisystem-Wakeup, selektiver Sync, Ignore, Konflikte, Limits, serverseitig widerrufender Logout und getrennter read-only FUSE-Mount | Aktueller Debian-Paketinhalt auf Ubuntu in realer GUI gestartet: Login, nativer Ordnerdialog, automatischer beidseitiger Sync, Ignore, Logout 1→0 aktive Serversitzungen und FUSE bestanden; isoliertes `dpkg` Install/Remove bestanden, systemweiter Lauf mangels Root offen |
| Linux Cloud-Mount | stabiler read-only Baum, On-Demand-Cache und klarer Offlinefehler | Strukturtests grün; echter Paket-GUI-`/dev/fuse`-Lauf war `ro`, Schreiben scheiterte, uncached Offline-Lesen lieferte I/O-Fehler ohne leere Datei und online vollständigen Inhalt |
| Windows Desktop | gemeinsamer Sync-Core und Tauri-Oberfläche | Rust-CFAPI-Code für `x86_64-pc-windows-gnu` kompiliert; echter Windows-Lauf offen |
| Windows Files-on-Demand | Sync-Root, Platzhalter, geprüfte Hydration, Upload bei Dateischluss, Offline behalten, sichere Dehydration und UI | Cross-Compile grün; Explorer-, Neustart- und Deinstallationstest auf Windows offen |
| Android | Kotlin/Compose-App, Dateien/Suche/Vorschau, Upload/Download, Share-Target, Shares, Papierkorb, Versionen, Offline-Inhalte, Kamera-Backup mit Android-14-Auswahlzugriff, DocumentsProvider, expliziter SAF-Ordner-Upload-Sync und kontrollierte Nutzer-CA-Unterstützung für Homelabs | API-37-Build, Lint und signierte RSA-3072-Release-APK grün; vier Tests auf Android-16-/API-36-Emulator mit Springfield-Nutzer-CA bestanden: 401/Login/Spaces, Kernnavigation sowie bereinigter Mehrblock-Upload/-Download, Versionen, Freigabe, Papierkorb und Restore; Hell/Dunkel und 130 % Schrift geprüft; physische Geräte-, Hersteller- und Hintergrundtests offen |
| Webhooks/API | versionierte API, Protokollkompatibilität, authentifiziertes Live-Wecksignal, HMAC-SHA256, stabile Event-ID und Retry-Outbox | Long-Poll-Wakeup sowie echte lokale Webhook-Zustellung und Signaturprüfung bestanden |
| Lastmessung | 2.000 kleine Dateien plus 128-MiB-Datei über echten lokalen HTTP-Server, danach Download in getrennte Clientwurzel | Upload 132,603 s, Download 164,905 s, 41.716 KiB Max-RSS auf dokumentierter Testhardware |
| Voller Datenträger | Blockschreib- und SQLite-`FULL`-Fehler liefern HTTP 507; Teilblöcke werden entfernt, bestätigte Versionen bleiben lesbar | Reales 8-MiB-`tmpfs`: 12-MiB-Änderung abgelehnt, Server weiter erreichbar, Versions-ID und Inhalts-Hash unverändert |
| Abhängigkeitssicherheit | reproduzierbare Lockfiles für Rust, Web und Desktop | RustSec: 291 Abhängigkeiten ohne Advisory; beide Produktions-NPM-Audits ohne Fund |
| Springfield-Deployment | Docker Compose, Caddy-HTTPS, Loopback-only Backend, separater Live-/Backup-Datenträger, 1.660 kuratierte Projektdateien und schreibgeschützte Release-Downloads | Upload/Download mit vollständigem Hashvergleich, Blockwiederverwendung, echter Netzabbruch mit Resume, Container-Neustart, Backupprüfung, Restore in leeren Zweitserver, Desktop-/Mobile-Weblauf sowie hashgleicher APK-/Deb-/CA-Download bestanden |
| Migrationen | idempotente Schemafolge bis Version 12 | Upgrade vorhandener v6-Daten mit Sitzung, Datei und Historie automatisiert; Schema 11→12 mit 1.660 Springfield-Bestandsdateien nach vorherigem Backup real ausgeführt |

Nicht abgeschlossen sind die ausdrücklich geforderten echten Windows- und
physischen Android-Abnahmeläufe, die systemweite Linux-Paketinstallation/-deinstallation
und ein Langzeittest. Deshalb ist der Stand weiterhin keine freigegebene
Version 1.
