# Android-Gestaltungsregeln

Die Android-App übernimmt die ruhige, warme Sprache der Web-App, bleibt aber
bei nativen Material-3-Komponenten und Android-Navigationsmustern.

- Farben: warmer Hintergrund `#FAF8F5`, weiße Flächen, Text `#292524`,
  Sekundärtext `#6B625A`, Akzent `#9A4F19`, Rahmen `#DED8D1`.
- Dunkelmodus: Hintergrund `#171513`, Fläche `#1C1A18`, Text `#EEEAE5`,
  Akzent `#D47A39`.
- Abstände folgen 4/8/12/16/24/32/40/48 dp. Formulare verwenden 14 dp zwischen
  Feldern und mindestens 48 dp hohe Bedienelemente.
- Karten gruppieren nur zusammengehörige Inhalte, verwenden 8–12 dp Radius und
  höchstens einen feinen Rahmen; es gibt keine dekorativen Schatten oder
  Verläufe.
- Formularfelder haben dauerhafte Beschriftungen oberhalb des Feldes,
  kontrastreiche Inhalte und direkte Fehlermeldungen. Während einer Anfrage
  bleiben eingegebene Werte lesbar und erhalten.
- Die Hauptaktion ist pro Ansicht eindeutig. Lade-, Fehler-, Leer- und
  Offlinezustände werden mit Text statt ausschließlich durch Farbe erklärt.
- Animationen bleiben bei den nativen Material-Zustandswechseln; zusätzliche
  dekorative Bewegung ist nicht vorgesehen.

Die übrigen bestehenden Datei-, Freigabe- und Einstellungsansichten verwenden
bereits dieselben semantischen Material-Farben. Ihre dichten Aktionsdialoge
sollten bei einer späteren visuellen Überarbeitung auf kleinen Displays erneut
gegen große Schriftstufen geprüft werden.
