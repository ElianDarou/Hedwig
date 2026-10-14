# Betrieb auf Ubuntu

## Voraussetzungen

- aktuelles Ubuntu LTS mit Docker Engine und Compose-Plugin
- eigener DNS-Name, der auf den Server zeigt
- HTTPS-Reverse-Proxy (Beispiel für Caddy unter `deploy/Caddyfile.example`)
- separates Backup-Ziel; ein Verzeichnis im Cloud-Datenbestand ist ungeeignet

## Installation (Entwicklungsstand)

```bash
git clone --branch v0.1.0 https://github.com/ElianDarou/Hedwig.git hedwig
cd hedwig
sudo install -d -o 10001 -g 10001 -m 0700 data backups
docker compose build
docker compose up -d
curl --fail http://127.0.0.1:8080/health
```

Danach wird `https://cloud.example.net` geöffnet. Solange kein Konto existiert,
zeigt die Web-App die einmalige Einrichtung des ersten Administrators. Der
Server besitzt keine voreingestellten Zugangsdaten.

Der Container lauscht auf dem Host nur auf `127.0.0.1:8080`. Der Betreiber muss
HTTPS am Reverse-Proxy konfigurieren; unverschlüsseltes HTTP darf nicht nach
außen freigegeben werden. Die UID/GID `10001` gehört dem unprivilegierten
Containerprozess; ohne passende Eigentümerschaft kann er in bind-gemountete
Daten- und Backupverzeichnisse nicht schreiben.

## Datenverzeichnisse

- `data/metadata.db`: SQLite-Metadatenbank mit WAL-Dateien
- `data/blocks/<space>/<prefix>/<hash>`: inhaltsadressierte Blöcke

Die Verzeichnisrechte müssen den Zugriff anderer lokaler Nutzer verhindern.
Hedwig aktiviert keine Betriebssystem- oder Datenträgerverschlüsselung.

## Update

Das Schema besitzt fortlaufende Migrationen. Vor einem Update wird trotzdem ein
geprüftes Backup erstellt:

```bash
docker compose exec hedwig hedwigctl backup \
  --data-dir /var/lib/hedwig --destination /var/backups/hedwig/hedwig-$(date +%F)
docker compose exec hedwig hedwigctl verify-backup \
  --source /var/backups/hedwig/hedwig-2026-09-20
```

Das Ziel muss ein neues Verzeichnis außerhalb des Live-Datenbestands sein. Ein
advisory Lock verhindert, dass Wartung und Snapshot gleichzeitig Blöcke
entfernen. Mit `HEDWIG_BACKUP_DIR`, `HEDWIG_BACKUP_INTERVAL_HOURS` und
`HEDWIG_BACKUP_KEEP` führt der Server konsistente, anschließend geprüfte
Sicherungen selbst aus und entfernt alte Sicherungsverzeichnisse nach der
konfigurierten Anzahl. Status, Fehler, Ziel, Warnung bei gleichem Dateisystem
und „Jetzt sichern“ stehen in der Administration. Alternativ kann `hedwigctl
backup` weiterhin von einem Host-Timer oder NAS-Job aufgerufen werden.

## Wiederherstellung und Export

Ein Restore schreibt ausschließlich in ein neues oder leeres Verzeichnis und
prüft vorher Manifest, BLAKE3 jedes Eintrags und SQLite:

```bash
hedwigctl restore --source /var/backups/hedwig/hedwig-2026-09-20 \
  --destination /srv/hedwig-restored
```

Der automatisierte Test startet anschließend einen frischen Server und lädt
eine Datei mit der wiederhergestellten Sitzung. Ein unabhängiger normaler
Verzeichnisexport ist möglich:

```bash
hedwigctl export --data-dir /var/lib/hedwig \
  --destination /export/hedwig-2026-09-20 --include-pinned
```

Aktuelle Dateien liegen pro Space unter `current/`, angeheftete Fassungen unter
`versions/`. Alle Blöcke werden dabei erneut gehasht.

## Wartung

Der Server führt standardmäßig alle sechs Stunden die Aufbewahrung aus. Manuell:

```bash
hedwigctl maintenance --data-dir /var/lib/hedwig
```

Offene Uploads schützen ihre Blöcke. Abgelaufene Uploads werden geschlossen,
Papierkorbobjekte nach 30 Tagen entfernt, Versionen ausgedünnt und nicht mehr
referenzierte physische Blöcke gelöscht. Mit
`HEDWIG_MAINTENANCE_INTERVAL_HOURS=0` wird der interne Zeitplan abgeschaltet.

## Voller Datenträger

Kann ein Block oder die SQLite-Transaktion wegen Speichermangel nicht dauerhaft
geschrieben werden, antwortet der Server mit HTTP 507 und bestätigt keine neue
Version. Bereits bestätigte Dateien bleiben lesbar; der reine Aktivätszeitstempel
eines Geräts darf in diesem Zustand ausfallen. Temporäre Teilblöcke werden nach
einem Schreibfehler entfernt.

Der Betreiber sollte weitere Uploads pausieren und zuerst Platz auf genau dem
Live-Datenvolume schaffen oder das Volume vergrößern. Blockdateien dürfen nicht
manuell nach Namen gelöscht werden. Nach der Entstörung bereinigt `hedwigctl
maintenance` abgelaufene Uploads und nicht mehr referenzierte Blöcke; noch offene
Uploads schützen ihre bereits empfangenen Blöcke absichtlich für eine Fortsetzung.

## Optionale Nur-Lese-Spiegelkopie

`HEDWIG_MIRROR_DIR=/var/mirror/hedwig` aktiviert einen gewöhnlichen
Verzeichnisbaum der aktuellen Dateien. Der Server ersetzt ihn atomar, setzt
Dateien und Verzeichnisse schreibgeschützt und zeigt Aktualisierungsstand sowie
zusätzlichen logischen Speicherverbrauch an. Das Intervall wird mit
`HEDWIG_MIRROR_INTERVAL_MINUTES` eingestellt; `0` deaktiviert nur den Zeitplan.
Der Spiegel ist kein zweiter Schreibzugang und kein Backup.

## Upload-Sperren und Diagnose

Administratoren können separate serverweite Verbotsmuster nach Gitignore-
Semantik pflegen. Sie gelten für Geräte- und Gast-Uploads und sind unabhängig
von lokalen `.cloudignore`-Regeln. Der Diagnoseexport enthält Systemzähler,
Backupstatus und das Ergebnis der Datenbankintegritätsprüfung, aber keine
Tokens, Passwörter, Dateipfade oder Dateiinhalte.

## Noch nicht als betriebsbereit freigegeben

Echte Windows- und physische Android-Zielsystemtests, eine systemweite Installation und
Deinstallation des Linux-Pakets, ein produktionsnaher Compose-Update-Test und
Langzeit-/Volllastprüfungen fehlen. Der aktuelle Linux-Paketinhalt wurde in der
echten Desktop-Sitzung mit Sync und FUSE ausgeführt. Trotzdem ist diese Fassung
nicht für die alleinige Aufbewahrung wichtiger Daten freigegeben.
