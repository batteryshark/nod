# macOS answer reachability regression

Run `python3 Tests/AppLayout/run.py --output /absolute/scratch` with installed
Xcode. Output must be outside the checkout; generated source, binary, module
cache and geometry JSON are not test sources and must not be committed.

The runner copies actual production models, detail, selection container, media,
markdown and status views. It derives the root layout from NodRootViews.swift,
excluding lifecycle/URL/alert modifiers so no real store is initialized. Only
the registered content is replaced with inert sidebar/list columns. The real
selection container resolves a synthetic request from the stub pending inbox.
Every write/network stub traps; windows are offscreen and never ordered front.

Checks cover all option IDs, order, native constrained scrolling, full button
containment and status-bar occlusion (not just document bounds). Footer geometry
is measured at intrinsic height, never assumed to be 44 points. Seven cases
cover 2/12/20 short/long options, long bodies, minimum/large/resized split views
and narrow standalone detail. Reverting the root repair fails the split cases.
For a negative control, supply `--root-source /absolute/baseline-root.swift`.

This is geometry regression, not a real-app visual, keyboard/VoiceOver, large
text, collapsed-column or required-text sheet interaction test. It does not
activate buttons, submit answers, load remote media or inspect user data.
