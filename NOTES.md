# Notes

Findings and decisions that are not obvious from the code. Measurements
are from kicad-cli 10.0.8 plots.

## Grid and font

- KiCad 10's default stroke font advances every glyph by the same amount:
  0.86755 x the font width ('i' and 'M' alike; glyph pairs measured 2.54 mm
  apart at width 1.4639). With width 1.4639 mm one character advances
  exactly 1.27 mm, one grid column. The height stays 1.27 mm, so text looks
  about 15% wider than KiCad's default; that is the price of exact column
  alignment.
- A cell is 1.27 mm wide (the schematic grid) and 2.54 mm tall (pin pitch,
  and roughly a terminal character's aspect). Wires, pins and labels sit on
  cell centers, which are all on the 1.27 mm connection grid.
- Text anchors differ by item type. Fields and symbol text with centered
  vertical justify put the capitals' center 0.09 mm above the anchor;
  schematic text items (`text`) 0.72 mm above it. The writer offsets each
  anchor so the capitals are centered on the row line; `tsch verify`
  re-measures this on every build, worst case so far 0.08 mm.
- KiCad's SVG export draws text as filled outlines that already include the
  stroke, and wraps each text in `<g class="stroked-text"><desc>TEXT`; the
  ink box is the bounding box of those paths. That is what verify checks.

## Symbols

- Every symbol is generated from the drawing: box outlines, glyph graphics
  and all pin text. Native pin names and numbers are hidden and drawn as
  symbol text instead, because KiCad centers a pin number on the pin line,
  which cannot match a number typed against the box edge, and draws top
  and bottom pin names rotated. The pins keep their real names and numbers
  for the netlist and ERC.
- Identical drawings share one library symbol (two identical regulators
  give one symbol). Names come from the value for boxes, from the glyph
  form for glyphs (`R_H2`, `C_V`), with a suffix when two drawings differ.
- The symbols are also written to `tsch.kicad_sym` with a `sym-lib-table`
  entry; without them ERC reports `lib_symbol_issues` on every part. Symbol
  library files carry their own format version (20251024), different from
  the schematic's (20260306); a library written with the schematic version
  does not load.
- Pin numbers are 1.0 mm text raised 0.76 mm above the pin line, so they
  stay inside the upper half of their row.
- Global labels use the passive shape. KiCad sizes the flag from the font
  width; with the default `label_size_ratio` 0.375 flags on adjacent rows
  overlap by 0.05 mm, so the project file sets 0.25, which leaves them
  0.27 mm apart and starts the name about 0.27 mm into the `[` cell.
- Local labels are written `(justify left)` without `bottom`, so the name
  sits beside the wire end on the row line instead of above the wire. The
  name starts about 0.2 mm right of the anchor, inside the wire's end cell.
- The project file is merged, not overwritten: tsch sets only
  `power_pin_not_driven = ignore` (rails from connectors or through
  passives have no power_out pin) and the label ratios.

## Parsing rules and why

- Text versus wire inside a run of characters: a run with a letter or digit
  is text, except dash runs that are wire: two or more dashes, a dash
  against a `[` or `]`, and leading or trailing dashes. A single leading or
  trailing dash with nothing beyond it stays text, so `-5V` and `USB_D-`
  survive; `USB_D-` left of a wire cannot, and needs `[USB_D-]`.
- `|` never belongs to text, so a vertical wire next to text is always seen.
- Crossings: a line drawn straight through another (`-|-` with `|` above
  and below) crosses without connecting, matching KiCad's no-dot rule; a
  T without `*` is an error rather than a guess.
- Field attachment went through three versions. Blocks of overlapping
  lines merged a value with the next part's reference (`BZX5V1` over `C7`).
  A one-row reach also caught text one blank row away. The current rule:
  each line attaches by touching the body (same row within 2 blank columns,
  or the adjacent row over its columns); diagonal positions do not count;
  stray lines join by left-aligned stacking. Beside a vertical glyph its
  pin stub rows count, but only when nothing touches more directly. A value
  line aligned under another part's field is an error, because the first
  version silently gave it to the neighbor.
- Label scope, measured against KiCad's netlist: on one sheet a local
  label joins a global label or power port of the same name (KiCad warns
  `same_local_global_label`); a local label never joins a name that exists
  only on another sheet. The first netlist model kept local and global
  apart, and `verify` caught the mismatch.
- Ties between parts are broken by the reference letter only when a line
  reaches more than one part; values like `SS14` or `LM358` look like
  references, so they cannot be used as an attachment filter on their own.
- One mistake, one error: a misplaced `x` on a box pin used to produce
  three errors; a broken box outline produced 79 in the first trial
  (TRIALS.md). Box pins now report the most specific problem only. A top
  edge with a matching bottom edge but a broken side reports `E102` at the
  broken cell, sorts it first, and parses the box as closed.
- Top and bottom pins are found by a `|` or `x` in the row just outside the
  edge, which is also where a value line sits. An `x` there counts only
  standing alone with an alphanumeric pin number after a space; otherwise
  `Conn_01x04` above a box became an unused pin.

## Tooling

- inkscape aborts on this GNOME setup (`org.gnome.settings-daemon.plugins.
  xsettings` has no key `antialiasing`) when it can reach the display;
  `render` runs it with `DISPLAY` and `WAYLAND_DISPLAY` removed.
- A 648-part A0 sheet (19,440 texts) checks in 0.2 s and verifies in 20 s,
  most of it kicad-cli. The text checks use 5 mm spatial buckets; the
  first all-pairs version took a minute.
- Rust `\` line continuations strip the next line's leading spaces, which
  silently shifts test drawings; tests use raw strings.
