# Springfield: realer Deployment- und Leistungsnachweis

Stand: 22. September 2026. Alle Werte in diesem Dokument wurden gegen den
laufenden Host `springfield` gemessen. Passwörter, Tokens, Recovery-Codes und
Dateiinhalte sind nicht Bestandteil des Berichts.

## Ergebnis

Hedwig läuft unter `https://springfield.fritz.box:8443` hinter Caddy. Der
Anwendungsport 8400 ist nur an das Loopback-Interface des Servers gebunden und
vom Laptop nicht direkt erreichbar. Der Anwendungscontainer ist gesund, die
SQLite-Datenbank verwendet Schema 12 und die für den Browsertest angelegte
Sitzung wurde serverseitig wieder beendet.

Der produktive persönliche Space enthält 1.660 kuratierte Quelldateien aus den
Projekten Hedwig, Runback, cliQuickRender und Emira. Das sind 37.087.455 Byte
aktuelle Inhalte. Git-Historien, Abhängigkeiten, Buildausgaben, Umgebungsdateien,
Schlüssel, Datenbanken, Aufzeichnungen und andere erkannte Geheimniskandidaten
wurden vor dem Upload ausgeschlossen. Alle 1.660 heruntergeladenen Dateien
stimmten im SHA-256-Vergleich; der Hash des sortierten Baummanifests war
`034e4097c0d9310bbdf25a546fa22c1f52ac4eaca28e893860875c8729187150`.

## Gemessene Übertragung

| Ablauf | Ergebnis |
|---|---:|
| 1.660 Projektdateien hochladen | 67,648 s; 0,523 MiB/s logischer Durchsatz |
| 1.660 Projektdateien herunterladen | 71,620 s; 0,494 MiB/s logischer Durchsatz |
| 128-MiB-Zufallsdatei erstmals hochladen | 27,501 s; 134.715.610 Byte Container-RX; 4,654 MiB/s |
| Reale 1-MiB-Änderung derselben Datei | 6,616 s; 1.360.786 Byte Container-RX |
| Einsparung der geänderten großen Datei | 98,99 % gegenüber dem Erstupload |
| Upload nach lokalem Rename im Upload-only-Modus | 39.818 Byte Container-RX; Inhalt durch Space-Deduplizierung nicht erneut übertragen |

Der Projektbaum besteht überwiegend aus kleinen Dateien. Dessen Durchsatz ist
daher nicht mit dem sequenziellen 128-MiB-Lauf vergleichbar. Die Werte sind eine
Messung auf diesem Host und kein allgemeines Geschwindigkeitsversprechen.

## Unterbrechung und Wiederaufnahme

Ein separater 128-MiB-Upload wurde nach 92.672.869 empfangenen Bytes durch das
gezielte Trennen des SSH-Transporttunnels unterbrochen. Der Agent endete sichtbar
mit einem Verbindungsfehler und meldete den Lauf nicht als synchronisiert. Mit
derselben lokalen Zustandsdatenbank übertrug der nächste Lauf nur noch 42.084.128
Byte und schloss in 12,857 s erfolgreich ab. Zusammen waren es 134.756.997 Byte;
bereits bestätigte Inhaltsblöcke wurden somit nicht nochmals als vollständige
Datei gesendet. Die anschließend heruntergeladene Datei hatte auf beiden Seiten
SHA-256 `b12128a6e08cdbaab7043217337483a3f92a5fe2d8080394857f0f320c4e3c70`.

Der Test deckte zusätzlich einen Fehler beim Löschen temporärer Spaces auf:
offene Uploads und aktuelle Dateizähler blieben erhalten. Die Serverlogik,
Migration 12 und ein Regressionstest korrigieren dies. Nach dem Deployment
meldete Springfield 0 offene Uploads, 1.660 aktuelle Dateien und 37.087.455 Byte
aktuelle Inhalte. Die normale 30-Tage-Sicherheitsfrist hält Blöcke gelöschter
Test-Spaces noch vor; deshalb sind physischer Blockverbrauch und historische
Versionssumme vorübergehend größer als die aktuellen Inhalte.

## Backup, Restore und Neustart

Vor der Migration wurde eine Sicherung geschrieben. Nach dem Update erzeugte
der Zeitplan erneut eine Sicherung mit 2.494 Einträgen und 310.845.529 Byte auf
dem vom Live-Datenträger getrennten Systemdatenträger. `hedwigctl verify-backup`
prüfte Manifest, alle Dateihashes und SQLite erfolgreich.

Diese Sicherung wurde in ein leeres temporäres Verzeichnis zurückgespielt und
als zweiter, isolierter Server gestartet. Der Restore meldete dasselbe Schema,
dieselben 1.660 aktuellen Dateien, 37.087.455 aktuelle Byte und 0 offene
Uploads. `Projekte/Hedwig/README.md` war bytegleich zum zuvor heruntergeladenen
Original. Testcontainer und Restore-Verzeichnis wurden danach entfernt. Ein
normaler Container-Neustart des produktiven Servers erhielt ebenfalls alle
Daten; vier Projekt-READMEs wurden danach erneut bytegleich abgerufen.

## Browserprüfung

Die echte HTTPS-Web-App wurde mit Chrome gegen Springfield geprüft: Anmeldung,
Projektordner, Administration, Backupstatus und Abmeldung funktionierten. Ein
erster 390-px-Lauf fand eine auf schmalen Displays ausgeblendete Abmeldung. Die
mobile Navigation wurde korrigiert, neu deployed und wiederholt geprüft. Bei
1.440 px und 390 px waren `innerWidth`, `clientWidth` und `scrollWidth` jeweils
identisch; es gab keinen horizontalen Seitenüberlauf. Im erfolgreichen Lauf
traten keine JavaScript-, Seiten- oder HTTP-Fehler auf. Der serverseitige
Sitzungszähler der abgemeldeten Browserinstanz sank auf 0. Screenshots liegen
unter `artifacts/springfield-web-admin.png` und
`artifacts/springfield-web-mobile.png`.

## Installationsartefakte

Springfield liefert vier fest benannte Dateien unter `/install/` schreibgeschützt
aus: die signierte Android-APK, das Ubuntu-/Debian-Paket, das öffentliche
Homelab-CA-Zertifikat und `SHA256SUMS`. Ein externer Abruf vom Laptop über die
installierte CA lieferte HTTP/2 200, `Cache-Control: no-store`,
`Content-Disposition: attachment` und die erwarteten Inhaltslängen. Alle drei
Prüfsummen bestanden nach dem erneuten Download. `/install/` selbst sowie eine
nicht vorhandene Datei lieferten 404; eine Verzeichnisauflistung ist nicht
aktiviert.

Die Release-APK ist 15.407.942 Byte groß und hat SHA-256
`f2bf22a38f53c73e1c4c5eb5f95ff0ce45d88da9640d7319d1a257f0f73bfacc`.
`apksigner verify` bestätigte APK-Signatur v2, genau einen Unterzeichner und ein
RSA-3072-Zertifikat mit SHA-256
`255ed02ad1ca108e241631e9bd4e78479ec27e3a2fa6f3929f7482c19b28f9a8`.
Der private Signierschlüssel liegt ausschließlich geschützt auf dem Buildhost
und wurde nicht auf Springfield übertragen. Das Debian-Paket ist 9.073.356 Byte
groß und hat SHA-256
`cdc8c548160ddf6d659c6df835cf7ff45c5749234b74c8039c13ea4f65778433`.
Ein erzwungener Neubau reproduzierte alle 142 APK-ZIP-Einträge einschließlich
ihrer Metadaten bytegleich und ergab erneut eine gültige Signatur mit demselben
Zertifikat. Nur der neu erzeugte v2-Signaturblock änderte den äußeren APK-Hash;
die hier genannte Prüfsumme gehört zum unverändert veröffentlichten Artefakt.

## Host und Grenzen der Messung

- Ubuntu 22.04.5 LTS, Kernel 5.15.0-186, 4 logische CPUs, 11 GiB RAM.
- Live-Daten: ext4 auf Samsung-SATA-SSD `/data` (476,9 GiB).
- Backups: ext4 auf dem separaten NVMe-Systemdatenträger.
- HTTPS verwendet im Heimnetz Caddys interne CA. Das öffentliche Root-Zertifikat
  liegt lokal unter `~/.config/hedwig/springfield-root.crt` und hat SHA-256
  `cfefc21b5218bafe6b45d85228f44cbfdd2da47eeb170ec7743d99150afaea5f`.
- Browser und native Clients müssen dieser CA ausdrücklich vertrauen. Android
  akzeptiert bewusst vom Gerätebesitzer installierte Nutzer-CAs, aber das Root-
  Zertifikat muss auf dem Telefon installiert werden.
- Lokal lief ein hardwarebeschleunigter Android-16-/API-36-Emulator über ADB.
  Mit installierter Springfield-Nutzer-CA bestanden falscher und richtiger
  Login, Space-Abruf, die zentralen App-Bereiche und ein bereinigter echter
  Datenfluss mit Mehrblock-Upload, Download-Bytevergleich, Versionen, Freigabe,
  Papierkorb und Wiederherstellung. Ein physisches Android-Gerät sowie
  herstellerspezifische Hintergrund- und Medienpfade wurden nicht geprüft.
- Windows Files-on-Demand wurde in diesem Lauf nicht auf Windows geprüft.
