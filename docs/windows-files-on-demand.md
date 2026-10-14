# Windows Files-on-Demand

Der Windows-spezifische Provider verwendet die Cloud Files API (CFAPI), die ab
Windows 10 Version 1709 für Desktop-Anwendungen verfügbar ist. Der aktuelle
Code registriert einen eindeutigen Sync-Root, legt Explorer-Platzhalter an,
verbindet einen `FETCH_DATA`-Callback und übergibt erst nach der bestehenden
Block- und Hashprüfung Daten an den Windows-Filter.

Grundlage sind die offiziellen Microsoft-Schnittstellen:

- [Hedwig Engines](https://learn.microsoft.com/en-us/windows/win32/cfapi/cloud-files-api-portal)
- [CfRegisterSyncRoot](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfregistersyncroot)
- [CfConnectSyncRoot](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfconnectsyncroot)

Die Registrierung verwendet eine progressive Hydration und erlaubt automatische
Dehydration. Die Datei-ID des Servers liegt als opaque Placeholder-Identität vor;
sie ist keine Berechtigung. Der Callback lädt weiterhin nur über einen
authentifizierten `CloudClient`. Ein Netzfehler wird als
`STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE` an Windows gemeldet und nicht als leerer
Inhalt bestätigt.

Beim Schließen einer lokal geänderten Platzhalterdatei liest der Provider eine
stabile Fassung ein und lädt sie mit Datei- und Basisversions-ID hoch. Ändert
sich die Serverfassung gleichzeitig, bleibt der Upload als Konfliktfassung
erhalten. „Offline behalten“ setzt den Pin und hydriert vollständig. „Speicher
freigeben“ verweigert die Dehydration, solange Windows die Datei nicht als
vollständig synchronisiert meldet.

Die Tauri-Oberfläche kann Files-on-Demand-Wurzeln anlegen, starten, stoppen und
entfernen sowie einzelne Dateien offline halten oder sicher freigeben. Profile
werden beim Start erneut verbunden; das Entfernen deregistriert den Sync-Root
und löscht keine bereits geladenen Nutzdaten.

## Noch ungeprüft beziehungsweise unvollständig

Der Provider ist für `x86_64-pc-windows-gnu` kompiliert, aber noch nicht auf
Windows ausgeführt. Vor einer Freigabe fehlen insbesondere:

- inkrementelle Aktualisierung des Platzhalter-Namensraums während einer
  laufenden Sitzung und aussagekräftige Explorer-Statussymbole,
- ein signierter Windows-Installer mit geprüftem Updatepfad,
- echte Tests auf unterstützten Windows-Versionen inklusive lokaler Änderung,
  Konflikt, Offline-Fehler, Neustart und Deinstallation.

Der vorhandene Compile-Check ist deshalb ausdrücklich keine Windows-Abnahme.
