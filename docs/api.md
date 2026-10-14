# HTTP API v1

Basis-Pfad: `/v1`. JSON-Antworten verwenden UTF-8. Geschützte Endpunkte
erwarten `Authorization: Bearer <device-session-token>`. Tokens werden nur bei
Einrichtung beziehungsweise Login ausgegeben und serverseitig gehasht.

Fehler haben die Form:

```json
{"error":{"code":"forbidden","message":"keine Berechtigung"}}
```

## Einrichtung und Anmeldung

- `GET /v1/setup/status`
- `POST /v1/setup` — einmalig erstes Administratorkonto und persönlicher Space
- `POST /v1/auth/login` — erstellt ein eigenes Gerät und eine Sitzung
- `POST /v1/auth/logout` — widerruft die aktuelle Sitzung
- `PUT /v1/auth/password` — prüft das alte Passwort und beendet andere Sitzungen
- `POST /v1/auth/recovery-codes` — ersetzt alle alten Codes; Klartext wird nur einmal ausgegeben
- `POST /v1/auth/recover` — verbraucht genau einen Wiederherstellungscode
- `GET /v1/auth/2fa`, `POST /v1/auth/2fa/begin|confirm|disable` — TOTP mit
  verschlüsseltem Secret; Login nimmt optional `otp` an
- `GET /v1/devices`, `DELETE /v1/devices/{device}`

Öffentliche Selbstregistrierung existiert nicht.

## Nutzer und Spaces

- `GET|POST /v1/admin/users` — nur Serveradministratoren
- `GET|POST /v1/admin/groups` sowie Gruppenmitglieder und Space-Gruppenrollen
- `GET|POST /v1/spaces`
- `PUT|DELETE /v1/spaces/{space}/members/{user}`
- `GET /v1/spaces/{space}/files?include_deleted=true`
- `GET|POST /v1/spaces/{space}/folders`, `POST /v1/folders/{folder}/move`,
  `DELETE /v1/folders/{folder}`
- `GET /v1/spaces/{space}/usage` — aktuelle Größen, Versionen, physischer Bedarf und Quote
- `PUT /v1/admin/users/{user}/quota`, `PUT /v1/admin/spaces/{space}/quota`
- `PUT /v1/admin/spaces/{space}/archive-protection`

Space-Rollen werden bei jeder Objektoperation serverseitig geprüft. Owner dürfen
nicht über die Mitgliedschafts-API ersetzt oder entfernt werden. Uploader können
nicht auflisten, lesen oder überschreiben; Namenskollisionen ihrer neuen Uploads
werden ohne Preisgabe des vorhandenen Inhalts aufgelöst.

## Fortsetzbarer Upload

1. `POST /v1/spaces/{space}/uploads` mit relativem Pfad, Dateigröße,
   vollständiger geordneter Blockliste, optionaler Datei-/Basisversions-ID und
   gerätespezifischem Idempotenzschlüssel.
2. Die Antwort nennt nur fehlende Block-Hashes.
3. `PUT /v1/uploads/{upload}/blocks/{blake3}` für jeden fehlenden Block. Größe
   und BLAKE3 werden geprüft; maximal 1 MiB im Inhaltsformat 1.
4. `POST /v1/uploads/{upload}/commit` bestätigt die vollständige Version in
   einer Metadatentransaktion. Wiederholung liefert dasselbe Ergebnis.

Ein Upload ist vor dem Commit nicht als Dateiversion sichtbar. Stimmt die
Basisversion nicht mehr, bestätigt der Server den Inhalt als Konfliktkopie.

## Dateien, Versionen und Papierkorb

- `GET /v1/files/{file}/content`
- `GET /v1/files/{file}/manifest` — autorisierte vollständige Blockliste zur
  Downloadprüfung
- `POST /v1/files/{file}/move` — Rename/Move im Space ohne Inhaltsupload
- `POST /v1/files/{file}/copy` — Kopie im selben oder in einen anderen Space;
  zwischen Spaces werden die autorisierten Inhaltsblöcke physisch getrennt
- `DELETE /v1/files/{file}` — idempotentes Verschieben in den Papierkorb
- `POST /v1/files/{file}/restore` — löst Namenskollisionen verlustfrei
- `GET /v1/files/{file}/versions`
- `GET /v1/files/{file}/versions/{version}/content`
- `POST /v1/files/{file}/versions/{version}/restore` — erzeugt eine neue aktuelle Version
- `PUT /v1/files/{file}/versions/{version}/pin` — benennt/heftet an oder löst die Anheftung
- `GET|POST /v1/spaces/{space}/folder-restore` — Vorschau und bestätigte
  Wiederherstellung eines Ordnerstands zu einem Zeitpunkt

Die Standardaufbewahrung wird durch die Wartung angewandt. Aktuelle und
angeheftete Versionen bleiben erhalten. Archivgeschützte Spaces werden weder
durch normale Clients gelöscht noch von der Versionsbereinigung reduziert.

## Freigabelinks

- `GET|POST /v1/shares`, `DELETE /v1/shares/{share}`
- `GET /v1/public/share`
- `GET /v1/public/share/files/{file}/content`
- `POST /v1/public/share/uploads`, Block-`PUT` und `commit`

Öffentliche Aufrufe übertragen das zufällige Geheimnis in
`Authorization: Share <token>`, nicht im URL-Pfad. Die Web-App legt es in das
URL-Fragment, das nicht an den Server gesendet wird. Passwörter werden mit
Argon2id gehasht. Upload-Links listen keine vorhandenen Dateien, erzwingen ihr
Größenlimit und überschreiben bei Namensgleichheit nichts. Widerruf und Ablauf
werden vor jedem Blockzugriff und erneut vor dem Commit geprüft.

## Change-Log

`GET /v1/changes?after=<sequence>&limit=<1..1000>` liefert berechtigte
Änderungen streng nach monotoner Sequenz. `next_cursor` darf erst nach lokaler
Verarbeitung dauerhaft bestätigt werden. Ereignis-IDs sind global eindeutig.

`GET /v1/events/wait?timeout_seconds=<1..30>` ist ein authentifiziertes,
inhaltsfreies Long-Poll-Wecksignal. Es kann nach erfolgreichen Mutationen früh
einen neuen Abgleich anstoßen, ersetzt aber niemals das Change-Log oder den
regelmäßigen Vollabgleich. Verlorene Signale beeinflussen daher nur die Latenz.

## Administration und Betrieb

- `GET /v1/admin/system`
- `GET /v1/admin/backup-status`, `POST /v1/admin/backup-run`
- `GET /v1/admin/mirror-status`, `POST /v1/admin/mirror-run`
- `GET|PUT /v1/admin/upload-policy` — serverweite Verbotsmuster nach
  Gitignore-Semantik
- `POST /v1/admin/integrity`
- `GET /v1/admin/diagnostics` — datensparsamer JSON-Export ohne Geheimnisse,
  Dateipfade oder Inhalte

## Webhooks

- `GET|POST /v1/spaces/{space}/webhooks`
- `DELETE /v1/spaces/{space}/webhooks/{webhook}`

Das Signiergeheimnis wird nur beim Erstellen ausgegeben. Zustellungen enthalten
`X-Cloud-Event-ID` und `X-Cloud-Signature: v1=<HMAC-SHA256>`. Fehler werden mit
exponentiellem Abstand erneut versucht; deshalb müssen Empfänger anhand der
stabilen Ereignis-ID idempotent sein. Erstellen, Ändern, Verschieben und Löschen
von Dateien sowie Erstellen/Widerrufen von Freigaben erzeugen Ereignisse.

## Freigabestatus

Die Schnittstelle ist versioniert und implementiert, aber noch nicht als
Version 1 freigegeben. Zielplattform-, Last- und Langzeittests aus dem
Prüfbericht stehen noch aus. Diese Datei dokumentiert den tatsächlich
implementierten Stand, nicht nur einen geplanten Vertrag.
