# ivaCAM

[English](./README.md) | **Deutsch**

**DXF- und SVG-Zeichnungen in G-Code für CNC-Fräsen, Laser, Plasmaschneider und Schleppmesser verwandeln.**

Freie, quelloffene CAM-Software für die Hobbywerkstatt. Eine eigenständige App für Desktop und Android — kein Python, keine Installationshölle — plus ein optionaler selbst gehosteter Web-Dienst und ein vollständig im Browser laufender (WebAssembly-)Modus, in dem Ihre Zeichnungen Ihren Rechner nie verlassen.

![preview](./assets/ivaCAM_GH_social.png)

## Verwendung

**Wollen Sie einfach etwas schneiden?** → [Schnellstart](./docs/QUICKSTART.md)

- **Desktop** (Linux / macOS / Windows): AppImage / `.msi` / `.app` bauen — siehe [Aus dem Quellcode bauen](#aus-dem-quellcode-bauen).
- **Android**: eine native APK (Tauri Mobile) bauen und auf dem Gerät installieren — siehe [Aus dem Quellcode bauen](#aus-dem-quellcode-bauen).
- **Browser**: den statischen Build ausliefern und die URL öffnen. Nichts zu installieren; alles läuft clientseitig.

## Funktionsumfang

- **Import**: DXF, SVG.
- **Operationen**: Profilieren, Tasche, Bohrung (Bohrzyklen), V-Carve, Gravur/Text, Anfasen, Gewinde, Schwalbenschwanz, T-Nut — mit Anbindungen, Lead-in/-out und Werkzeugversätzen.
- **Maschinen**: Fräse, Laser, Plasma (Einstechen + Verweilen), Schleppmesser — jede mit eigenem Postprozessor.
- **Vorschau**: 2D-Zeichenfläche plus ein Live-3D-Werkzeugweg und eine Materialabtrag-Simulation.
- **Werkstatt-Einrichtung**: Werkzeugbibliothek (Parameter pro Werkzeug), wiederverwendbare Maschinenprofile, Projekt speichern/laden.

## Aus dem Quellcode bauen

```sh
cargo build --workspace                        # Core + CLI + Server
cd frontend && pnpm install && pnpm dev        # Web-UI auf :5173
cargo tauri build --bundles appimage           # Desktop-Bundle
cargo tauri android build --apk                # Android-APK
```

Plattformspezifische Voraussetzungen und der vollständige Ablauf: [docs/BUILDING.md](./docs/BUILDING.md) (Windows: [docs/BUILDING_WINDOWS.md](./docs/BUILDING_WINDOWS.md), Android: [docs/BUILDING.md § Android](./docs/BUILDING.md)).

## Dokumentation

- [Schnellstart](./docs/QUICKSTART.md) — von der Zeichnung zum G-Code in 5 Minuten
- [Bauen](./docs/BUILDING.md) — jeden Transport bauen & paketieren
- [Architektur](./docs/ARCHITECTURE.md) · [Mitwirken](./docs/CONTRIBUTING.md) — zum Weiterentwickeln

## Lizenz

GPL-3.0-or-later — siehe [`LICENSE`](./LICENSE). ivaCAM erbt diese Lizenz aus seiner teilweisen Ableitung von viaConstructor (GPLv3); siehe Danksagungen.

## Danksagungen

Baut auf hervorragenden Open-Source-Bibliotheken auf — [`dxf-rs`](https://github.com/IxMilia/dxf-rs), [`cavalier_contours`](https://github.com/jbuckmccready/cavalier_contours), `clipper2-rust`, [`usvg`](https://github.com/linebender/resvg), [Svelte](https://svelte.dev/), [Three.js](https://threejs.org/) und [Tauri](https://tauri.app/).

Der CAM-Kern von ivaCAM ist teilweise eine Rust-Portierung der Geometrie- und Werkzeugweg-Routinen von [viaConstructor](https://github.com/multigcs/viaconstructor) (`calc.py`, `machine_cmd.py`, `setupdefaults.py`) und wird entsprechend unter der GPL vertrieben. [Estlcam](https://www.estlcam.de/) hat Teile des Funktionsumfangs und der Terminologie inspiriert, ist aber Closed-Source und steuert keinen Code bei.
