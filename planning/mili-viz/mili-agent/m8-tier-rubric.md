# M8 tier rubric — concrete examples per `intent_class`

**Status (2026-05-25):** Stage B4 draft. Frozen once the M8 pilot run
(Stage E) confirms the tier definitions produce actionable failure
buckets. Update only with a corresponding bench re-run.

Companion artifact to
[`m8-corpus-distillation.md`](m8-corpus-distillation.md) §"Three
difficulty tiers". Where the parent doc defines the tier abstractly
("Easy = single tool call with canonical names"), this file pins the
**concrete shape per intent_class** so the teacher's prompt has an
unambiguous target for each (tier × intent_class) cell.

The 18 supported intent_classes are the entries in `tools.json` plus
the M7 Delta 5 `list_results` lookup, plus compound families that
chain ≥2 typed tools.

---

## Definitions (recap)

- **Easy.** Single tool call. The user prompt uses the canonical
  argument shape directly. No lookup. Termination is one tool call +
  one short ack. ≤ 5 model-turn tokens.
- **Intermediate.** Either (a) the user phrasing requires a
  `list_results` lookup before the productive call, or (b) the prompt
  is a 2-step compound chain whose arguments are all canonical.
- **Hard.** Multi-step compound chain AND natural-language phrasing,
  AND/OR a recovery branch after an initial failure (typo → fail →
  `list_results` → corrected call).

---

## Per-intent_class examples

The table below pins one *exemplar tool sequence* per (tier ×
intent_class) cell. The synth generator MUST produce scenarios whose
tool sequence shape matches the cell's exemplar — variation is in the
*phrasing* and *argument values*, not in the tool sequence skeleton.

| intent_class | Easy | Intermediate | Hard |
| --- | --- | --- | --- |
| `load` | "load d3samp6" → `load{root:d3samp6}` → ack | (rare — lookup doesn't apply to load) | "open the cylinder one and then go to the last state" → `load{root:cylinder}` → `step{dir:LAST}` → ack |
| `close` | "close the database" → `close{}` → ack | (rare) | "close out and reset the view" → `close{}` → `view{reset:true}` → ack |
| `set_state` | "set state 50" → `set_state{state:50}` → ack | (rare — direct integer is canonical) | "jump to the state with the highest principal stress" → `list_results` → ... → `set_state` (oracle-grounded) → ack |
| `step` | "next" → `step{dir:NEXT}` → ack | "go to the last state" → `step{dir:LAST}` → ack | "step forward 3 times" → `step{dir:NEXT}` ×3 → ack |
| `select` | "select brick 1-10" → `select{class_name:brick,range:"1-10"}` → ack | "select the first ten bricks" → `select{class_name:brick,range:"1-10"}` → ack | "select bricks 1-10 and clear the shell selection" → `select{...}` → `clrsel{class_name:shell}` → ack |
| `clrsel` | "clear selection" → `clrsel{class_name:""}` → ack | "clear the brick selection" → `clrsel{class_name:"brick"}` → ack | (rare — clrsel is single-shot) |
| `show` | "show vx" → `show{result:vx}` → ack | "show the first principal stress" → `list_results` → `show{result:prin_stress1}` → ack | "show the von Mises stress at the deformed configuration" → `list_results` → `show{result:eff_stress}` → `view{reset:true}` → ack |
| `view` | "reset the view" → `view{reset:true}` → ack | "zoom in 2×" → `view{zoom:2.0}` → ack | "save the current view as 'front' then reset" → `named_view{op:SAVE,name:front}` → `view{reset:true}` → ack |
| `iso` | "isosurface vx at 5 levels" → `iso{result:vx,on:true,count:5}` → ack | "show isosurfaces of effective stress" → `list_results` → `iso{result:eff_stress,on:true,count:5}` → ack | "show 3 isosurfaces of effective stress and disable material 2" → `list_results` → `iso{result:eff_stress,on:true,count:3}` → `material{enable:false,material:2}` → ack |
| `contour` | "10 contours of vx" → `contour{result:vx,count:10}` → ack | "10 contours of the y-velocity" → `list_results` → `contour{result:vel_y,count:10}` → ack | (compound) |
| `material` | "disable material 2" → `material{enable:false,material:2}` → ack | "hide the brick class" → `material{enable:false,class_name:"brick"}` → ack | "hide materials 2 and 3 then show stress" → `material` ×2 → `show` → ack |
| `cutplane` | "set a cut plane at origin x=0 normal z" → `cutplane{...}` → ack | (params resolved from natural language) | "clear the cut plane and show eff_stress" → `cutplane{...clear...}` → `show{result:eff_stress}` → ack |
| `colormap` | "use jet colormap" → `colormap{name:jet}` → ack | "use the rainbow colormap" → `colormap{name:jet}` (mapped via aliases) → ack | (compound) |
| `legend` | "set legend 0 to 100" → `legend{min:0,max:100}` → ack | "autoscale the upper bound" → `legend{min:0}` → ack | (compound) |
| `named_view` | "save view as 'front'" → `named_view{op:SAVE,name:front}` → ack | (rare) | "save view as 'front', restore 'side', then list views" → `named_view` ×3 → ack |
| `query` | "query sx for brick 1" → `query{result:sx,class_name:brick,labels:[1],states:[<current>]}` → ack | "query the x-velocity for the first ten nodes" → `list_results` → `query{result:vel_x,class_name:node,labels:[1..10],states:[<current>]}` → ack | "for each of states 10, 50, 90, query the von Mises stress on brick 1" → `query` ×3 → ack |
| `snapshot` | "snapshot the session" → `snapshot{}` → ack | (rare) | (rare) |
| `griz_raw` | (rare; reserve for the escape hatch) | (rare) | "run `mat color 1 red`" → `griz_raw{line:"mat color 1 red"}` → ack |
| `list_results` | (the model never asks for this directly — it's a lookup tool. Skip.) | n/a | n/a |
| `compound-material-then-show` | n/a (compound is by definition ≥2 steps) | "disable material 2 then show eff_stress" → `material{enable:false,material:2}` → `show{result:eff_stress}` → ack | "disable material 2 and show first principal stress at state 81" → `material` → `list_results` → `show` → `set_state` → ack |

---

## Tier distribution within a production run

Per the M8 plan, 40 / 40 / 20 (easy / intermediate / hard). The
generator should produce roughly proportional counts within each
intent_class — overweight `show` and `set_state` (the highest-leverage
intents) by ~1.5×, underweight the rare `griz_raw` / `cutplane` /
`legend` cells.

Stratify reporting by tier in every bench summary so a future
regression doesn't hide under an averaged-pass-rate.
