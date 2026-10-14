# Hedwig auf Ubuntu installieren

Version 0.1.0 ist eine Vorabversion. Die Release-Dateien sind für Ubuntu 24.04
LTS auf einem Intel-/AMD-Rechner mit 64 Bit (`uname -m`: `x86_64`) vorgesehen.
Windows-, ARM- und Android-Installer gehören nicht zu diesem ersten Release.

## Desktop-App

Die Desktop-App verbindet sich mit einem vorhandenen Hedwig-Server. Falls du
noch keinen hast, starte zuerst den Server wie im nächsten Abschnitt.

```bash
mkdir -p ~/Downloads/hedwig-0.1.0
cd ~/Downloads/hedwig-0.1.0
curl -fLO https://github.com/ElianDarou/Hedwig/releases/download/v0.1.0/hedwig-desktop_0.1.0_amd64.deb
curl -fLO https://github.com/ElianDarou/Hedwig/releases/download/v0.1.0/SHA256SUMS
sha256sum --check --ignore-missing SHA256SUMS
sudo apt update
sudo apt install ./hedwig-desktop_0.1.0_amd64.deb
```

Die Prüfsummenprüfung muss `OK` melden. `apt` installiert die benötigten
WebKitGTK-, GTK- und Tray-Bibliotheken mit. Öffne danach **Hedwig** im
Anwendungsmenü. Trage die vollständige Serveradresse, deinen Benutzernamen und
dein Passwort ein und wähle einen Space und einen lokalen Sync-Ordner.

Das Schließen des Fensters lässt Hedwig im Tray weiterlaufen. Autostart kann
in der App aktiviert werden. Für den optionalen, nur lesbaren Cloud-Mount wird
zusätzlich der FUSE-Helfer benötigt:

```bash
sudo apt install fuse3
```

## Server auf demselben Rechner ausprobieren

Das Serverarchiv enthält Server, Verwaltungswerkzeug, Sync-Agent, FUSE-CLI und
die fertig gebaute Web-App. Rust, Node.js und Docker werden dafür nicht benötigt.

```bash
mkdir -p ~/Downloads/hedwig-0.1.0
cd ~/Downloads/hedwig-0.1.0
curl -fLO https://github.com/ElianDarou/Hedwig/releases/download/v0.1.0/hedwig-server_0.1.0_linux_amd64.tar.gz
curl -fLO https://github.com/ElianDarou/Hedwig/releases/download/v0.1.0/SHA256SUMS
sha256sum --check --ignore-missing SHA256SUMS
mkdir -p ~/.local/opt
tar -xzf hedwig-server_0.1.0_linux_amd64.tar.gz -C ~/.local/opt
mkdir -p ~/.local/share/hedwig-server
chmod 700 ~/.local/share/hedwig-server
~/.local/opt/hedwig-server_0.1.0_linux_amd64/bin/hedwig-server \
  --data-dir ~/.local/share/hedwig-server \
  --web-dir ~/.local/opt/hedwig-server_0.1.0_linux_amd64/web \
  --listen 127.0.0.1:8080
```

Lass das Terminal offen und rufe <http://127.0.0.1:8080> im Browser auf.
Die Web-App führt dich durch die einmalige Einrichtung des Administrators.
Es gibt kein Standardpasswort. Anschließend kannst du die Desktop-App mit
`http://127.0.0.1:8080` und dem eingerichteten Konto verbinden.

`Strg+C` beendet den Server. Derselbe Startbefehl verwendet beim nächsten Mal
die vorhandenen Daten. Für dauerhaftes Hosting mit HTTPS, automatischen Backups
und Docker Compose siehe [Betrieb auf Ubuntu](operations.md). Der lokale
Start oben bindet ausschließlich an den eigenen Rechner.

## Updates und Deinstallation

Ein neues Desktop-Paket wird mit `sudo apt install ./<neues-paket>.deb`
installiert. Sichere die Serverdaten vor einem Serverupdate; Details stehen in
der [Betriebsanleitung](operations.md).

```bash
sudo apt remove hedwig
```

Deinstallation entfernt die Desktop-Anwendung. Deine Sync-Ordner und die
Serverdaten bleiben erhalten.

## Aus dem Quellcode bauen

Server und Web-App benötigen Rust 1.94 oder neuer sowie Node.js 22 oder neuer:

```bash
git clone --branch v0.1.0 https://github.com/ElianDarou/Hedwig.git
cd Hedwig
cargo test --locked --workspace
cargo build --locked --release --workspace
npm ci --prefix web
npm run build --prefix web
```

Der Desktop-Paketbau benötigt Docker mit BuildKit/Buildx:

```bash
docker build -f desktop/Containerfile.bundle --target artifact \
  --output type=local,dest=dist/desktop .
```

Die Paketierung folgt der
[Tauri-Anleitung für Debian-Pakete](https://v2.tauri.app/distribute/debian/).
