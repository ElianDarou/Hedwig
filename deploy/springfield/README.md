# Springfield-Deployment

Dieses Ziel veröffentlicht Hedwig ausschließlich über Caddy unter
`https://springfield.fritz.box:8443`. Der direkte Serverport ist nur auf dem
Loopback-Interface des Hosts erreichbar.

Live-Daten liegen auf `/data/hedwig`; automatische Backups liegen mit
`/home/elian/services/hedwig/backups` auf dem anderen physischen Datenträger.
Beide Verzeichnisse müssen der Container-UID 10001 gehören und Modus 0700 haben.

Caddy verwendet für das reine Heimnetz eine interne CA. Deren öffentliches
Root-Zertifikat liegt nach dem ersten Start unter
`caddy-data/caddy/pki/authorities/local/root.crt` und muss auf jedem Client als
vertrauenswürdige lokale CA installiert werden. Der private CA-Schlüssel darf
den Server nie verlassen. Ohne diese Vertrauenseinrichtung melden native
Clients und Browser erwartungsgemäß einen Zertifikatsfehler.

Fertige Installationsdateien werden schreibgeschützt in den Caddy-Container
eingebunden und können im Heimnetz heruntergeladen werden:

- Ubuntu/Debian: `https://springfield.fritz.box:8443/install/hedwig-linux-amd64-0.1.0.deb`
- Benchmark-Bericht: `https://springfield.fritz.box:8443/install/hedwig-benchmark.html`
- öffentliche Homelab-CA: `https://springfield.fritz.box:8443/install/springfield-root.crt`
- Prüfsummen: `https://springfield.fritz.box:8443/install/SHA256SUMS`

Ein neues Android-Release wird erst veröffentlicht, wenn es mit dem bisherigen
Hedwig-Signaturschlüssel signiert werden kann. Die vorhandene APK-Signatur muss
für sichere Updates derselben App-Identität erhalten bleiben; ein Debug-Schlüssel
oder ein neu erzeugter Release-Schlüssel ist dafür kein Ersatz. Der aktuelle
Android-Quellstand wurde gebaut und im Emulator getestet, aber nicht als APK
auf Springfield bereitgestellt.

Die bereits vorhandenen Dateien mit dem alten `cloud-sync-`-Namen bleiben als
historische Downloads erhalten; die neue Hedwig-Desktopdatei ersetzt sie nicht.

Das CA-Zertifikat muss vor der Installation über einen bereits vertrauenswürdigen
Kanal geprüft werden. Sein SHA-256-Fingerabdruck lautet
`cfefc21b5218bafe6b45d85228f44cbfdd2da47eeb170ec7743d99150afaea5f`.
Der erste Abruf derselben CA über eine noch nicht vertrauenswürdige Verbindung
ist allein kein sicherer Vertrauensanker.

Auf Android wird `root.crt` über die Systemeinstellung zum Installieren eines
CA-Zertifikats für VPN und Apps eingespielt. Die App vertraut zusätzlich zum
Android-Systemspeicher bewusst dem vom Gerätebesitzer verwalteten Zertifikat-
speicher; HTTP bleibt gesperrt. Das erweitert den Vertrauensbereich auf alle
vom Gerätebesitzer installierten CAs und ist deshalb nur für kontrollierte
Geräte vorgesehen.

```bash
docker compose -f deploy/springfield/compose.yaml config --quiet
docker compose -f deploy/springfield/compose.yaml build
docker compose -f deploy/springfield/compose.yaml up -d
curl --cacert ~/services/hedwig/caddy-root.crt \
  https://springfield.fritz.box:8443/health
curl --cacert ~/services/hedwig/caddy-root.crt \
  https://springfield.fritz.box:8443/install/SHA256SUMS
```

Der tatsächlich ausgeführte Deployment-, Transfer-, Browser- und Restore-Test
ist in [`docs/springfield-verification.md`](../../docs/springfield-verification.md)
dokumentiert.
