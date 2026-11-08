# Controller prompts

92 selected images from Kenney Input Prompts 1.5A, CC0; see [the license](LICENSE.md).
The source pack is supplied locally under `extra_assets/kenney_input_prompts/`.

Run `python3 tools/prepare_controller_prompts.py` from the repository
to copy the PNGs here and regenerate `embedded.rs`. Raw PNGs stay ignored under
the repository's existing text-only publication policy. The embedded exact PNG
bytes are portable and independent of the viewer's converted-content asset root.
Each image has a source filename and SHA-256 comment. No proprietary target
artwork is included here.

The family and glyph order matches `Family::ALL` and the prompt rows in the
viewer controller module: Xbox, PS5, Deck, Steam Controller; then South, East,
West, North, LB, RB, LT, RT, Start, Select, Left stick, Right stick/pad, D-pad, then D-pad Up, Down, Left, Right; directional right stick/pad Right, Up, Left, Down;
sequence arrow and combination plus.
