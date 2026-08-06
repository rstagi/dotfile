# `world/` — the per-talk world layer (this is where your deck stops looking generic)

The engine (`hooks/`, `components/core/`, the generic scenes, the token scaffold)
is shared across every talk. **The world is not.** This directory is empty on
purpose — it's the extension point where you build the metaphor that makes *this*
talk memorable.

A world is made of three additive things — none of them touch the engine:

1. **New scene types.** Add a `type` to the `SceneType` union and a content
   interface in `src/data/slides.ts`, write a `XScene.tsx` here or in `scenes/`,
   and add a `case` to the `renderScene` switch in `App.tsx`.
2. **Hand-built SVG primitives** (this folder). Animated SVG components that read
   the `--talk-*` CSS variables and react to `subStep`/state — a throne whose
   occupant flips, a map marker that travels, an HP bar that drains. **Hand-built
   only — never an AI-generated or stock image.**
3. **A chrome badge.** Pass a `badge={…}` into `<DeckChrome>` in `App.tsx` to dress
   the top-right slot (the GoT deck put a "who holds the throne" sigil there).

Worked examples to study (full custom decks, same engine, totally different worlds):

- **Game of Thrones / "AIron Throne"** (`~/Dev/playground/aiewf-online-talk/`):
  `world/Throne.tsx` (occupant flips JS→Python→TS), `HouseSigil.tsx`, `Raven.tsx`;
  `RealmChrome` showed the throne-holder; sections were "Books".
- **Pokémon / RPG** (`~/Dev/playground/talk-indie-milano-20251218/`): a world map
  with terrain/paths/sprites, HP bars, boss fights, loot drops, level-ups.

The invariant that keeps it reusable: scenes and primitives only ever reference
CSS variable **names** (`--talk-accent`, `--fs-*`, `--margin`) and the `cqw/cqh`
scale. Only the token **values** (in `index.css`) and this world layer change per talk.
