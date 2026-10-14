# Android-Client

Native Kotlin-/Compose-App für Android 8 und neuer.

```bash
ANDROID_HOME=/home/elian/Android/Sdk ./gradlew \
  :app:lintDebug :app:testDebugUnitTest :app:assembleDebug
```

Die Debug-APK liegt danach unter `app/build/outputs/apk/debug/app-debug.apk`.
Der aktuelle Build verwendet Android 17/API 37, AGP 9.4.0, Gradle 9.7.1 und
bleibt ab Android 8/API 26 installierbar. Ein Release-Artefakt muss mit einem
Betreiberschlüssel signiert werden; der Debug-Schlüssel ist nicht zur Verteilung
bestimmt.

Der signierte Springfield-Build liegt unter
`artifacts/hedwig-android-0.1.0.apk` und wird im Heimnetz über
`https://springfield.fritz.box:8443/install/hedwig-android-0.1.0.apk`
angeboten. Seine SHA-256-Prüfsumme lautet
`f2bf22a38f53c73e1c4c5eb5f95ff0ce45d88da9640d7319d1a257f0f73bfacc`.
`apksigner verify` bestätigt eine APK-Signatur v2 mit einem RSA-3072-Zertifikat;
dessen SHA-256-Fingerabdruck lautet
`255ed02ad1ca108e241631e9bd4e78479ec27e3a2fa6f3929f7482c19b28f9a8`.
Der private Signierschlüssel liegt nicht im Projekt oder auf dem Webserver.
Implementiert sind HTTPS-Anmeldung, verschlüsselte Sitzung im Android Keystore,
Space-/Dateiansicht, Suche, Upload über Dateiauswahl und Teilen-Menü,
WorkManager-Retry mit WLAN-/Ladebedingungen, Offline-Dateien, Kamera-Backup für
gewählte MediaStore-Ordner sowie ein bedarfsgeladener, nur lesbarer
`DocumentsProvider`. Freigabelinks können aufgelistet, als Lese- oder reine
Upload-Freigabe mit Passwort, Ablauf und Größenlimit erstellt und widerrufen
werden.

Für Server mit öffentlichem Zertifikat ist keine zusätzliche Einrichtung
nötig. Bei einer privaten Homelab-CA muss deren Root-Zertifikat vom
Gerätebesitzer über Androids Systemeinstellungen als CA-Zertifikat für VPN und
Apps installiert werden. Die App akzeptiert solche Nutzer-CAs bewusst, blockiert
aber weiterhin unverschlüsseltes HTTP. Dadurch können alle auf dem Gerät
installierten Nutzer-CAs App-Verbindungen beglaubigen; auf nicht kontrollierten
Geräten sollte deshalb ein öffentlich vertrauenswürdiges Zertifikat verwendet
werden.

Für Springfield wird zuerst `springfield-root.crt` installiert. Dessen SHA-256-
Fingerabdruck
`cfefc21b5218bafe6b45d85228f44cbfdd2da47eeb170ec7743d99150afaea5f`
muss über SSH oder einen anderen bereits vertrauenswürdigen Kanal geprüft werden.
Danach kann die APK heruntergeladen werden. Android verlangt dabei je nach
Hersteller die ausdrückliche Freigabe „Unbekannte Apps installieren“ für den
verwendeten Browser oder Dateimanager. Diese Berechtigung kann nach der
Installation wieder entzogen werden.

Bei der ersten Anmeldung an Springfield muss als Serveradresse vollständig
`https://springfield.fritz.box:8443` und als Benutzername `elian` eingetragen
werden. Der Kurzname `springfield` wird von Android nicht zuverlässig per DNS
aufgelöst; `elianUser` ist kein eingerichtetes Konto. Das Passwort ist das
aktuelle Springfield-Kontopasswort, der 2FA-Code bleibt ohne aktivierte 2FA leer.

Ein mit Androids Systemdialog ausdrücklich freigegebener Ordner kann zusätzlich
periodisch oder manuell in einen gewählten Space hochgeladen werden. Dieser
SAF-Ordnerzugriff ist bewusst „nur hochladen“: neue und geänderte Dateien werden
mit Datei- und Basisversions-ID übertragen, Konflikte bleiben erhalten, lokale
Löschungen löschen keine Cloud-Dateien. Die App behauptet keinen Zugriff auf
andere Ordner. Das ist kein beidseitiger Android-Dateisystem-Sync.

Android darf Hintergrundarbeit verzögern. Die App zeigt das offen und garantiert
keinen sofortigen Upload im geschlossenen Zustand. Sie fordert keinen
uneingeschränkten Dateisystemzugriff an. Der DocumentsProvider liefert bei
fehlendem Netz ohne Cache einen Fehler statt einer leeren Datei.

Die App wurde auf einem hardwarebeschleunigten Android-16-/API-36-Emulator
installiert und ausgeführt. Die private Springfield-CA wurde als Nutzer-CA
installiert. Geprüft wurden das Anmeldeformular, sichtbare 401-Fehler, der echte
HTTPS-Login, das Laden der Spaces sowie Offline, Freigaben, Papierkorb und
Einstellungen. Ein separater bereinigter Springfield-Datenfluss prüfte
Mehrblock-Upload, idempotente Wiederholung, bytegleichen Download, eine zweite
Version, Versionshistorie, passwortgeschützte Freigabe mit Widerruf sowie
Löschen und Wiederherstellen. Heller und dunkler Modus sowie 130 % Schriftgröße
wurden visuell geprüft. Nicht geprüft bleiben ein physisches Gerät,
herstellerspezifische MediaStore-Berechtigungen, unterbrochenes Kamera-Backup
und stark eingeschränkte Hintergrundausführung.
