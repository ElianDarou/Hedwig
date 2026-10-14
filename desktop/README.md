# Desktop-Client

Der Tauri-Client verwendet direkt `hedwig-agent`; es gibt keine zweite
Synchronisationslogik. Gerätezugänge liegen im Credential Store des
Betriebssystems, während nicht geheime Ordnerprofile atomar im App-Konfigurations-
verzeichnis gespeichert werden.

## Entwicklung

```bash
npm ci
npm run build
npm run tauri dev
```

Unter Ubuntu werden die von Tauri dokumentierten WebKitGTK-/GTK-
Entwicklungspakete benötigt. Ein reproduzierbarer Backend-Check steht bereit:

```bash
docker build -f desktop/Containerfile.check -t hedwig-desktop-check .
```

Ein installierbares Debian-Paket wird reproduzierbar in einem Container gebaut:

```bash
docker build -f desktop/Containerfile.bundle --target artifact \
  --output type=local,dest=desktop/bundle-output .
sudo apt install ./desktop/bundle-output/*.deb
```

Die Deinstallation des Pakets entfernt die Anwendung, aber weder ausgewählte
Sync-Ordner noch deren Nutzdaten.

Das Schließen des Fensters blendet es aus; der Abgleich läuft im Tray weiter.
„Entfernen“ löscht nur das Sync-Profil, niemals den ausgewählten Nutzerordner.

Die Desktop-Oberfläche enthält außerdem Aktivitäten, eine lokale
Konfliktübersicht, einen verlustfreien Editor für lokale Ignore-Regeln mit
Regelherkunft und Vorschau, Autostart, konfigurierbare Löschschwellen sowie eine
ausdrückliche Einmalfreigabe angehaltener Massenlöschungen. Selektiver Sync ist
von Ignore-Regeln getrennt und sichert lokal geänderte Dateien außerhalb des
Sync-Ordners, bevor ein Cloud-Ordner lokal entfernt wird. Upload- und
Downloadlimits lassen sich getrennt einstellen; `0` bedeutet unbegrenzt.

Unter Linux kann ein getrennter leerer Ordner als ausschließlich lesbarer
Online-Zugriff eingehängt werden. Der FUSE-Mount lädt Inhalte erst beim Öffnen,
prüft die Serverblöcke, hält einen Inhaltscache und liefert ohne Netz für noch
nicht geladene Dateien einen I/O-Fehler. Ein Pfad kann nicht zugleich Mount und
klassischer Sync-Ordner sein. Der Mount lässt sich in der App ein- und
aushängen; beim Abmelden werden aktive Mounts beendet.

Windows Files-on-Demand ist an die Oberfläche angeschlossen und cross-kompiliert,
aber noch nicht auf Windows ausgeführt. Noch offen für die vollständige
Desktop-Abnahme sind signierte Installer, eine automatische Updateauslieferung,
der echte Windows-Durchlauf und eine systemweite Debian-Installation samt
Deinstallation. Ein isolierter `dpkg`-Install-/Remove-Lauf ist bestanden. Der
aktuelle Debian-Paketinhalt wurde auf diesem Linux-Host in der echten
Desktop-Sitzung mit Anmeldung, beidseitigem Live-Sync, Ignore, serverseitigem
Logout sowie dem echten Kernel-FUSE-Mount geprüft.
