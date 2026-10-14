# Hedwig

Hedwig ist eine selbst gehostete Datei-Cloud. Dieses Repository enthält
einen fortgeschrittenen, aber noch nicht als Version 1 freigegebenen
Entwicklungsstand der Produktspezifikation vom 19. September 2026.

Der aktuelle Stand ist bewusst als **Entwicklungsstand** gekennzeichnet. Eine
grüne Build-Pipeline bedeutet nicht, dass die plattformspezifischen
Abnahmekriterien bereits erfüllt sind. Der genaue Stand steht in
[`docs/coverage.md`](docs/coverage.md).

Eine verständliche Zusammenfassung der Sync-Leistungsoptimierungen steht in
[`docs/performance-optimierungen.md`](docs/performance-optimierungen.md).

## Installation auf Ubuntu

Der erste öffentliche Release ist **v0.1.0 (Vorabversion)**. Das Desktop-Paket
ist für Ubuntu 24.04 LTS mit x86_64/amd64 vorgesehen:

```bash
curl -fLO https://github.com/ElianDarou/Hedwig/releases/download/v0.1.0/hedwig-desktop_0.1.0_amd64.deb
sudo apt update
sudo apt install ./hedwig-desktop_0.1.0_amd64.deb
```

Danach **Hedwig** im Anwendungsmenü öffnen und mit der Adresse deines
Hedwig-Servers anmelden. Die Desktop-App benötigt einen laufenden Server.
Die [Ubuntu-Anleitung](docs/installation-ubuntu.md) beschreibt auch den lokalen
Serverstart, Prüfsummen, FUSE und die Deinstallation.

[Downloads und Release-Hinweise](https://github.com/ElianDarou/Hedwig/releases/tag/v0.1.0)

## Lizenz

GNU AGPL-3.0-only; der vollständige Lizenztext steht in [LICENSE](LICENSE).

## Repository

- `crates/hedwig-core`: gemeinsames Inhaltsblock-, Ignore- und Pfadverhalten
- `crates/hedwig-client`: typisierter, blockverifizierender HTTP-Client
- `crates/hedwig-agent`: persistenter Windows-/Linux-Sync-Agent
- `crates/hedwig-mount`: nur lesbarer Linux-FUSE-Online-Zugriff
- `crates/hedwig-files-windows`: Windows-CFAPI-/Files-on-Demand-Adapter
- `crates/hedwig-server`: einzelner HTTP-Server mit SQLite-Metadaten und Blockspeicher
- `crates/hedwigctl`: Diagnose- und Verwaltungswerkzeug
- `web`: responsive Web-App
- `desktop`: Desktop-Client (Tauri, Windows/Linux)
- `android`: native Android-App (Kotlin/Compose)
- `deploy`: Docker-Compose-Beispiel
- `docs`: Architektur, Betrieb, Protokoll und Prüfbericht

## Entwicklung

```bash
cargo test --workspace
cargo run -p hedwig-server -- --data-dir ./data --listen 127.0.0.1:8080
```

Der native Android-Client wird separat gebaut:

```bash
cd android
ANDROID_HOME=/home/elian/Android/Sdk ./gradlew :app:assembleDebug
```

Ein einmaliger Entwicklungsabgleich lässt sich ausführen mit:

```bash
HEDWIG_TOKEN='<device-token>' cargo run -p hedwig-agent -- \
  --server https://cloud.example.net --space '<space-id>' \
  --root /pfad/zum/sync-ordner --state-dir /pfad/zum/client-zustand
```

Bei einer privaten Homelab-Zertifizierungsstelle kann der Agent deren PEM-Datei
über `HEDWIG_CA_CERT=/pfad/zur/root-ca.pem` zusätzlich vertrauen. Die
Zertifikatsprüfung bleibt dabei aktiv; es gibt keinen unsicheren Abschaltmodus.

Der Token wird absichtlich nicht als normales Argument dokumentiert, damit er
nicht in der Prozessliste erscheint. Die Desktop-App speichert ihn im
Credential Store des Betriebssystems.

Beim ersten Start wird ein Einrichtungsstatus angeboten. Ein Administrator wird
nicht mit einem bekannten Standardpasswort angelegt.
