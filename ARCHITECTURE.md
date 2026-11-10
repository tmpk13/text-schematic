# Architecture

```mermaid
flowchart LR
    tsch[".tsch drawing<br/>+ TOML header"] --> source["source.rs<br/>split header, grid, ASCII check"]
    source --> meta["meta.rs<br/>part table, defaults"]
    source --> parse["parse.rs<br/>rules, sheet model"]
    meta --> parse
    glyphs["glyphs.rs<br/>two-pin forms and graphics"] --> parse
    parse --> model["model.rs<br/>parts, wires, labels,<br/>ports, nets"]
    manifest["project.toml"] --> project["project.rs<br/>sheets, cross-sheet nets"]
    model --> project
    project --> kicad["kicad.rs + sexpr.rs<br/>symbols, sheets,<br/>.kicad_pro"]
    kicad --> files[".kicad_sch, tsch.kicad_sym,<br/>sym-lib-table, .kicad_pro"]
    files --> cli["kicad-cli<br/>netlist, ERC, SVG"]
    cli --> verify["verify.rs<br/>netlist equality,<br/>text inside cells"]
    project --> verify
```

## Parse order

Each stage claims cells; a claimed cell is invisible to later stages, which
is how the one-element-per-cell rule is enforced.

```mermaid
flowchart TD
    A["boxes: closed + - | outlines"] --> B["glyphs: [==] || ~~ >| # === ) V"]
    B --> C["box pins: numbers against edges, padding, tips;<br/>pin names inside"]
    C --> D["x on pin tips: no-connects"]
    D --> E["text runs: split wire dashes from text"]
    E --> F["labels at horizontal wire ends;<br/>power ports at vertical wire ends"]
    F --> G["fields: lines touching a body,<br/>then left-aligned stacks"]
    G --> H["parts: reference, value, extras,<br/>pin types, glyph variants"]
    H --> I["wires: arms, junctions, crossings;<br/>union-find nets; KiCad wire runs"]
```

## Coordinates

Cell (row, col) has its center at x = 15.24 + col x 1.27 mm,
y = 15.24 + row x 2.54 mm on the page. Every symbol is placed at angle 0 at
its origin cell (a box's top-left corner, a glyph's first cell); symbol
graphics are generated y-up relative to that cell.
