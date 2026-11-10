# tsch: text schematics to KiCad

`tsch` turns a schematic drawn as plain text into a KiCad 10 schematic. One
character is one 1.27 x 2.54 mm cell of the sheet, so every part, wire and
text lands in KiCad exactly where it was typed; the checker enforces the
drawing rules and refuses anything that would overlap. It is the default
way Claude draws a new schematic, through the `kicad-schematic` skill of the
pcb plugin in claude-setup. The format is in [FORMAT.md](FORMAT.md)
(`tsch spec` prints it).

Day to day:

- `mise run install`: put `tsch` on PATH (a launcher that rebuilds from this checkout when the source changed).
- `mise run check`: check the example drawings.
- `mise run verify`: build the examples into `out/` and prove them with kicad-cli (netlist round trip, ERC, every plotted text inside its cells).
- `mise run render`: plot the feature tour to `out/tour/*.png`.
- `tsch check|build|verify|netlist|show|render FILE`: run the tool on your own drawing (`mise run tsch -- ...` before install).
- `mise run test`: unit tests and the KiCad round trip.

`mise tasks` lists the rest. Design notes are in [NOTES.md](NOTES.md), the
structure in [ARCHITECTURE.md](ARCHITECTURE.md), and results of agents
drawing with it in [TRIALS.md](TRIALS.md).
