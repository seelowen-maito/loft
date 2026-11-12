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
  there. A `*` matches any run of characters: `*.csproj` is any file ending in `.csproj`, `requirements*.txt`
  matches `requirements.txt` and `requirements-dev.txt`.
- **no markers** (`&[]`) is only for names that never hold anything but rebuildable files (`node_modules`,
  `DerivedData`). `.venv` needs a Python project beside it (`pyproject.toml`, `requirements*.txt`, `setup.py`,
  `setup.cfg` or `Pipfile`).

## Why the marker is required

The sweep deletes. A bare `build/`, `dist/`, `bin/` or `out/` may be someone's real files (a script folder, a
checked-in release), so such a name is build output only beside a file that proves a build tool owns it: `bin/`
beside a `.csproj`, `build/` beside `build.gradle` or `pubspec.yaml`. Never delete real files: when in doubt, ask
for a marker.

## Never a folder git tracks

Some teams commit `Pods/`, `Carthage/Build/`, `bin/` or even `node_modules/`. Before the sweep deletes a folder
inside a clone or worktree, it asks git whether it tracks any file in it (`git ls-files`). If git does, the
folder stays and the log lists it as skipped, "tracked by git". Git runs the way Clean's Check runs it
(`backend/src/git_check.rs`): only when the repo's config holds nothing outside a short allowlist, on exactly its
git folders, read-only, with a timeout. When git can't answer (an unusual config, an error, a timeout), the
folder stays too: "git can't say whether it tracks it".

The 3-day age rule, running agents, symlink handling and the tracked-folder check apply to every rule the same
way; a rule only says what is build output.

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
