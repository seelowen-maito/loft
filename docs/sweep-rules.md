# Teaching the sweep a new stack

The daily sweep and Clean's preview remove build output from the agents' workspaces. What counts as build
output is one table, `RULES` in `backend/src/sweep.rs`. Nothing else in the sweep names a build folder.

## A rule

```rust
rule("bin", DOTNET),            // folder, markers
rule("Carthage/Build", &["Cartfile"]),
rule("node_modules", &[]),
```

- **folder**: a folder name, or a path from its parent, like `ios/build` or `Carthage/Build`.
- **markers**: files that must sit in the folder's parent. The folder is build output only when at least one is
  there. `*.csproj` matches any file ending in `.csproj`.
- **no markers** (`&[]`) is only for names that never hold anything but rebuildable files (`node_modules`,
  `.venv`, `DerivedData`).

## Why the marker is required

The sweep deletes. A bare `build/`, `dist/`, `bin/` or `out/` may be someone's real files (a script folder, a
checked-in release), so such a name is build output only beside a file that proves a build tool owns it: `bin/`
beside a `.csproj`, `build/` beside `build.gradle` or `pubspec.yaml`. Never delete real files: when in doubt, ask
for a marker.

The 3-day age rule, running agents and symlink handling apply to every rule the same way; a rule only says what
is build output.

## Worked example: adding Elixir

Elixir builds into `_build/` and `deps/` beside `mix.exs`.

1. Add the rows to `RULES`:

   ```rust
   // Elixir
   rule("_build", &["mix.exs"]),
   rule("deps", &["mix.exs"]),
   ```

2. Add the cases to `CASES` in the tests of `sweep.rs`:

   ```rust
   ("_build", Some("mix.exs")),
   ("deps", Some("mix.exs")),
   ```

That is all. `every_rule_is_swept_beside_its_marker_and_kept_without_it` builds a clone for each case, checks the
folder is swept when its marker is beside it and kept when it isn't, and that source files are untouched. It also
fails when a rule in `RULES` has no case, so a row can't be added without one.

Out of scope: caches shared across workspaces (`~/.nuget`, `~/.gradle`, pnpm's store) and Android emulator images.
