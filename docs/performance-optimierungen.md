# So macht Hedwig den Dateiabgleich schneller

Stand: 23. September 2026

Diese Seite erklärt in einfachen Worten, welche Änderungen den Abgleich
beschleunigen sollen und was die bisherigen Messungen tatsächlich zeigen.
Geschwindigkeit wurde nicht auf Kosten der Dateiprüfungen erkauft.

## Kurz erklärt

- Viele Dateien werden gleichzeitig geprüft statt strikt nacheinander.
- Große Dateien werden in kleinere Teile zerlegt. Hochgeladen werden nur Teile,
  die auf dem Server noch fehlen.
- Viele kleine Datei- und Blockanfragen werden zu größeren Anfragen gebündelt.
- Downloads werden direkt auf die Festplatte geschrieben und dabei geprüft,
  statt erst die ganze Datei im Arbeitsspeicher zu sammeln.
- Dateien werden erst sichtbar, wenn Inhalt und Prüfsummen stimmen.

## Was wurde optimiert?

### Viele Dateien gleichzeitig prüfen

Der Sync-Agent sucht Dateien im Ordner und berechnet ihre Inhalts-Prüfsummen
parallel. So muss er einen Ordner mit vielen kleinen Dateien nicht Datei für
Datei abarbeiten. Während eine Datei gelesen wird, prüft Hedwig außerdem, ob sie
sich verändert hat. Bei einer Änderung wird der Lauf nicht mit einem veralteten
Inhalt bestätigt.

Auch bei einem Lauf ohne Änderungen liest Hedwig die Dateien weiterhin, um
Änderungen sicher zu erkennen. Es werden dann aber keine unnötigen Datei-Inhalte
über das Netz übertragen und keine leeren Datenbankänderungen gespeichert.

### Nur fehlende Dateiteile übertragen

Hedwig teilt Dateien in kleinere, inhaltsabhängige Blöcke. Jeder Block bekommt
einen schnellen Fingerabdruck. Das ist eine kurze Kontrollzahl aus dem Inhalt:
Ändert sich der Inhalt, ändert sich normalerweise auch diese Zahl. Die
Blockgröße passt sich dem Inhalt an: meist rund 256 KiB, mindestens 64 KiB und
höchstens 1 MiB. Vor dem Upload fragt der Client den Server, welche
Fingerabdrücke noch fehlen. Bereits vorhandene Blöcke werden nicht noch einmal
hochgeladen. Gleiche Inhalte werden dabei nur im selben gemeinsamen oder
persönlichen Speicherbereich (Space) wiederverwendet.

Intern heißt das Verfahren zum Finden der Blöcke FastCDC; für die
Fingerabdrücke verwendet Hedwig BLAKE3. Diese technischen Namen muss man bei
der normalen Nutzung nicht kennen.

Das hilft besonders, wenn nur ein Teil einer großen Datei geändert wurde. Ein
Umbenennen innerhalb eines Spaces verschiebt dagegen nur den Dateieintrag: Der
Inhalt wird nicht erneut übertragen.

### Übertragungen parallelisieren und bündeln

Ohne gesetztes Bandbreitenlimit können mehrere Übertragungen gleichzeitig
laufen. Die Desktop-Einstellung für gleichzeitige Downloads steht ab Werk auf
16 und lässt sich zwischen 1 und 64 einstellen. Uploads verwenden getrennte,
begrenzte Gruppen: bis zu acht fehlende Blöcke bei einer einzelnen Datei oder
bis zu vier Blockpakete bei mehreren Dateien gleichzeitig. Viele Dateien
werden zusätzlich als Upload-Gruppe angemeldet.

Mehr gleichzeitige Arbeit kann kurzzeitig mehr Prozessor- und Festplattenlast
erzeugen. Wer es ruhiger möchte, kann die Zahl der Übertragungen verringern.

Ein Gruppenaufruf kann bis zu 1.024 Dateien anmelden. Die Blockdaten werden in
Pakete mit höchstens 512 Blöcken und ungefähr 32 MiB Nutzdaten aufgeteilt. Das
verringert die Zahl einzelner Netzwerk-Anfragen, was vor allem bei vielen
kleinen Dateien hilft.

Sobald ein Upload- oder Download-Limit aktiv ist, laufen Downloads einzeln.
Upload-Blöcke laufen einzeln, wenn ein Upload-Limit aktiv ist. So vervielfacht
sich das eingestellte Limit nicht durch mehrere gleichzeitige Verbindungen.

Hinweis: Die Einstellung von 1 bis 64 steuert aktuell die Zahl gleichzeitiger
Downloads. Die Upload-Gruppen haben die oben genannten festen Grenzen.

### Weniger Wartezeit für Datenbank und Festplatte

Neue Sync-Zustände werden gesammelt und gemeinsam in die lokale Datenbank
geschrieben. Gibt es nichts zu speichern, entfällt diese Schreibtransaktion.
Auf dem Linux-Server werden viele Blockdateien gesammelt geschrieben und mit
gemeinsamen Festplatten-Sicherungsgrenzen dauerhaft gemacht. Das spart einzelne
langsame Wartevorgänge pro Block. Die Dateien bleiben trotzdem erst nach den
Prüfungen und dem sicheren Wechsel an ihren endgültigen Speicherort sichtbar.

### Große Dateien mit begrenztem Arbeitsspeicher bearbeiten

Uploads lesen Dateien blockweise; Downloads fließen direkt in eine temporäre
Datei. Hedwig muss dafür nicht die komplette Datei im Arbeitsspeicher halten.
Während des Lesens werden die Block-Prüfsummen und zusätzlich die Prüfsumme der
gesamten Datei kontrolliert.

### Änderungen schneller bemerken

Der Desktop-Client reagiert auf Dateisystem-Ereignisse und wartet zusätzlich
auf ein inhaltsfreies Wecksignal des Servers. Ereignisse werden kurz
zusammengefasst, damit eine Folge schneller Dateischreibvorgänge nicht viele
doppelte Läufe startet. Regelmäßige vollständige Abgleiche bleiben als
Sicherheitsnetz bestehen, falls ein Ereignis verloren geht.

## Sicherheit trotz höherer Geschwindigkeit

- Jeder Block wird vor der Speicherung geprüft. Die ganze Datei wird vor dem
  Abschluss erneut kontrolliert. Verändert sie sich während des Uploads, wird
  der Upload nicht als fertig bestätigt.
- Ein Download landet zunächst in einer temporären Datei. Erst wenn Größe,
  Blöcke und Gesamtinhalt stimmen, setzt Hedwig ihn an seinen endgültigen Platz.
- Der Server sichert Blöcke vor dem Datenbank-Commit. Server und Client nutzen
  für ihre SQLite-Datenbanken die volle eingestellte Schreibsicherheit.
- Wiederholte Anfragen erzeugen nicht einfach eine zweite Datei. Nach einem
  unterbrochenen Upload werden bereits bestätigte Blöcke wiederverwendet.
- Ein echter Netzabbruchtest unterbrach einen 128-MiB-Upload. Der nächste Lauf
  setzte mit den noch fehlenden Daten fort; die fertige Datei war anschließend
  auf beiden Seiten bytegleich.

## Was die bisherigen Messungen zeigen

Der aktuelle Springfield-Bericht vergleicht denselben Dateikorpus mit echten
Clients. Für Hedwig wurden drei frische Spaces verwendet; angegeben ist der
Median, also der mittlere Wert dieser Läufe. Der Kontrolllauf ohne Änderungen
umfasst neun Messungen.

Der Korpus hatte 907 Dateien mit zusammen 297.789.033 Byte. Die fertigen
Downloads aller drei Produkte hatten denselben geprüften Datei-Baum.

| Ablauf | Vorher | Aktueller Hedwig-Median | Veränderung |
|---|---:|---:|---:|
| Erster Upload | 55,74 s | 16,45 s | 70,5 % kürzer |
| Frischer Download | 37,07 s | 10,58 s | 71,5 % kürzer |
| 80 Alltagsänderungen | 6,68 s | 3,69 s | 44,7 % kürzer |
| Änderung an einer 192-MiB-Datei | 5,19 s | 1,46 s | 71,8 % kürzer |
| 192-MiB-Datei umbenennen | 4,21 s | 0,75 s | 82,2 % kürzer |
| Lauf ohne Änderungen | 4,19 s | 0,68 s | 83,8 % kürzer |

Bei der 4-MiB-Änderung in der großen Datei sendete Hedwig 4,41 MiB. Seafile
sendete 7,94 MiB; Nextcloud übertrug 192,70 MiB. Beim Umbenennen waren es bei
Hedwig nur etwa 3 KiB Steuer- und Metadatenverkehr, nicht der Dateiinhalt.

Im Vergleich mit den anderen Produkten war Hedwig in fünf von sechs Szenarien
schneller. Seafile war beim ersten Upload schneller: 12,59 s gegenüber Hedwigs
16,45 s. Bei den 80 Alltagsänderungen lagen Hedwig mit 3,69 s und Seafile mit
3,80 s dicht beieinander; Nextcloud benötigte 8,95 s.

## So sind die Zahlen einzuordnen

- Die Zeiten sind Messungen auf Springfield, keine Garantie für andere Rechner,
  Netzwerke oder Dateien.
- Für die veränderten Szenarien gab es je einen Lauf mit Nextcloud und Seafile;
  Hedwig wurde dreimal in frischen Spaces gemessen. Der Bericht zeigt deshalb
  Hedwig-Mediane und nennt die Einzelwert-Spannen.
- Eine Hedwig-Wiederholung überschnitt sich zeitweise mit hoher CPU-Last des
  Android-Emulators. Der dritte Lauf fand nach dem Abschalten des Emulators
  statt.
- Eine frühere Hedwig-Optimierungsrunde maß den Erstupload mit 14,10 s; der
  aktuelle Median liegt bei 16,45 s. Der schnellere Einzelwert ist also nicht
  bei jedem späteren Lauf garantiert.
- Die Messung belegt nicht, dass Windows oder Android dieselben Zeiten erreichen.
  Ihre echten Plattform- und Hintergrundtests sind gesondert zu bewerten.
- Ein separater Lasttest mit 2.000 kleinen Dateien und einer 128-MiB-Datei
  verwendete höchstens rund 41 MiB Arbeitsspeicher. Er lief auf anderer Hardware
  und ist nicht direkt mit dem Springfield-Vergleich vergleichbar.

Die vollständigen Messwerte, Vergleichstabellen und Einschränkungen stehen im
[Hedwig-Benchmark-Bericht](../benchmark/hedwig-benchmark.html). Der aktuelle
Prüfstand für die übrigen Produktbereiche steht in
[der Erfüllungsmatrix](coverage.md).

## Technische Stellen im Projekt

- [Sync-Agent: Suche, Prüfung und Übertragungsablauf](../crates/hedwig-agent/src/sync.rs)
- [HTTP-Client: Upload, Download und Blockprüfung](../crates/hedwig-client/src/lib.rs)
- [Server: gebündelte Upload-Anfragen](../crates/hedwig-server/src/api.rs)
- [Server: sicherer Blockspeicher](../crates/hedwig-server/src/storage.rs)
- [Benchmark-Daten und Berechnung](../benchmark/build_report_data.py)
- [Weitere ausgeführte Last- und Plattformtests](verification.md)
