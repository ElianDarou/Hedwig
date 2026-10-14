# Oberflächenregeln

Web und künftige native Clients verwenden eine sachliche Datei-Hierarchie statt
eines Karten-Dashboards. Die Oberfläche ist deutsch, Texte bleiben zentral
ersetzbar, und Statusmeldungen benennen den tatsächlichen Zustand.

- Abstände folgen 4/8/12/16/24/32 px beziehungsweise dp.
- Bedienelemente haben maximal 8 px Radius; Inhaltsflächen schweben nicht.
- Akzentfarbe ist warmes Braun/Orange, Fehlerfarbe Rot; Bedeutung wird immer
  zusätzlich durch Text vermittelt.
- Web unterstützt System-Hell/Dunkelmodus, Tastaturfokus und 320 px Mindestbreite.
- Native Touch-Ziele werden mindestens 48 dp groß.
- Sync-Zustände unterscheiden lokal, wartend, übertragend, bestätigt,
  ausgeschlossen, pausiert und Fehler. „Synchronisiert“ ist nur der bestätigte
  Endzustand.
- Bewegung bleibt auf kurze Zustandswechsel beschränkt; Reduced Motion wird
  respektiert.

Der Web-Rename verwendet derzeit einen einfachen Browserdialog; alle Aktionen
bleiben damit tastaturbedienbar, ein eigener Dialog wäre lediglich zusätzlicher
Feinschliff. Die Desktop-App verwendet einen festen Navigationsbereich, native
Ordnerdialoge und textlich benannte Sync-/Fehlerzustände. Android ist als native
Compose-App implementiert und folgt System-Hell-/Dunkelmodus sowie 48-dp-Zielen.
