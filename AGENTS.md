maki lua plugins extend maki with tools, commands, keybindings, and event handlers. This one registers the `multi_review` tool and the `/multi-review` command. It loads through the maki pack system, not as a builtin.

## Code guidelines

- Lua in `plugin/`. No trivial comments, minimal bloat, no unnecessary state.
- Fallible runtime operations return the `(value, err)` pair and never throw. Tool handlers fail with `{ llm_output = msg, is_error = true }`; a plain string is always success.
- Every tool that declares `permission_scopes` must also declare `permission`, and the capability must be granted in `plugin.toml`. maki refuses the load otherwise.
- `plugin.toml` grants exactly what the code calls. maki walks bundled plugins' lua and requires to keep manifests honest; keep ours aligned by hand. Nothing this plugin calls is gated, so `[permissions]` is empty on purpose.
- `min_maki_version` in `plugin.toml` is a separate floor from the maki revision the dev-dependencies pin. Before using an API the plugin does not already call, confirm it exists at the declared floor, or raise the floor.

## Configuration

The reviewer list comes from the `multireview.reviewers` slot and nothing else.

- A package gets no `setup(opts)` call, so there is no entry point for options.
- `maki.api.register_options` is for bundled plugins. maki rejects a `plugins.<name>` table for a plugin it does not ship and startup fails. Do not reach for it.
- Read the slot inside the tool handler. The chain is async and a layer may park, and the handler is a coroutine that can wait. Load time cannot, so reading it at the top level would break a parking layer.
- A layer that throws is skipped by design. Treat an unusable result as a configuration error and say so, rather than running zero reviewers in silence.

## Testing

`nix develop` provides the pinned Rust toolchain and every tool below. The stylua version in `flake.nix` and the `STYLUA_VERSION` in the workflow must stay equal, otherwise local formatting and CI disagree.

CI runs the same recipes in `.github/workflows/plugin.yml`. Cheapest first:

- `just check`
- `just lint`
- `just test`
- `just fmt-lua` writes the formatting `stylua --check plugin/` enforces in CI.

Tests live in `tests/` and load the plugin through the real `maki-lua` host with `PluginHost::load_package`, passing the repo root (it derives `plugin/` itself; passing `plugin/` fails with `PackageEmpty`).

Dev-dependencies pin a revision of maki. Move the pin when the host changes what the plugin uses.

Assert Lua-visible effects (callback output, mailbox messages), not just files. `smol::unblock` side effects land even when a callback aborts, so file-only checks can pass while the Lua-level API is broken.

## Layout

- `plugin/`, the plugin entry files, loaded sorted by file name. Chunks share one environment.
- `plugin.toml`, the `min_maki_version` floor and the `[permissions]` request.
- `tests/`, host harness and integration tests.
- `justfile`, check, lint, test, fmt-lua.
- `.gitignore` carries `!/AGENTS.md` to defeat a global exclude in `~/.config/git/ignore`. Removing that line drops this file from the repository.

## Docs

The README is the canonical home for install and usage. Follow the maki docs voice: plain words, no em-dashes, no contractions, state facts once.
