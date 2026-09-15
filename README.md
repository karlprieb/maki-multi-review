# maki-multi-review

A [maki](https://github.com/tontinton/maki) package that reviews your working diff from several angles at once. It spawns one read-only subagent per configured reviewer, in parallel, each pinned to its own model. Every reviewer reads the same summary of the changes and reports findings as `file:line - summary`. The main agent then merges the reports, drops duplicates, and groups what is left by theme.

Two reviewers on the same model with different angles is a real use. So is one cheap model and one expensive one on the same diff, to see what the cheap one misses.

## Install

Add the package to your global `~/.config/maki/init.lua`:

```lua
maki.pack.add({ "https://github.com/karlprieb/maki-multi-review" })
```

Maki asks once before installing, records the commit in `pack-lock.json`, and loads the package at startup. The plugin requests no permissions, so there is no second prompt.

To pin a release, pass a table instead of a bare string:

```lua
maki.pack.add({
  { src = "https://github.com/karlprieb/maki-multi-review", version = "v0.1.0" },
})
```

Run `/packupdate maki-multi-review` to move to a newer commit.

## Configure

No reviewer is configured by default, so the tool errors until you set a list. The list lives in the `multireview.reviewers` slot.

### Global

`~/.config/maki/init.lua` holds the list every project uses:

```lua
maki.api.set_slot("multireview.reviewers", function()
  return {
    { model = "deepseek/deepseek-flash" },
    { name = "security", model = "claude/claude-opus-5" },
    { name = "performance" },
  }
end)
```

### Per project

A trusted project's `.maki/init.lua` sets the same slot and wins, because project config loads after global config and the newest layer runs first. Returning a list without calling `prev` replaces the global one:

```lua
maki.api.set_slot("multireview.reviewers", function()
  return {
    { name = "concurrency", model = "claude/claude-opus-5" },
    { name = "api compatibility", model = "claude/claude-opus-5" },
  }
end)
```

To keep the global reviewers and add to them, call `prev` and append:

```lua
maki.api.set_slot("multireview.reviewers", function(prev)
  local list = prev()
  table.insert(list, { name = "sql injection", model = "claude/claude-opus-5" })
  return list
end)
```

The slot is read on every invocation, so `/reload` is enough after an edit. No restart.

### Reviewer options

An entry is a table. Both fields are optional, and an empty table `{}` is a valid reviewer that runs the current model with no angle.

| Field   | Type   | Default                     | What it does                                                                                                                        |
| ------- | ------ | --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `name`  | string | index plus model suffix     | The review angle. It goes into the reviewer's prompt ("Review these changes from a security angle") and becomes its section header. |
| `model` | string | the session's current model | An exact model spec, the same string `/model` shows, such as `claude/claude-opus-5`.                                                |

Without `name`, the generated name is the position in the list and the part of the model after the last slash, so `deepseek/deepseek-flash` at position 1 becomes `1-deepseek-flash`. That name still reaches the prompt, which is why a real angle is worth setting.

The section header shows the model next to the name only when you set `name`. Without it the generated name already carries the model.

## Use

### `/multi-review` with no argument

```text
/multi-review
```

The agent collects `git diff` and `git diff --cached`, summarizes both, and calls the tool. If both are empty, or the directory is not a git repository, it stops and tells you there is nothing to review instead of spawning reviewers on an empty diff.

### `/multi-review` with an instruction

```text
/multi-review focus on the error paths in the new parser
```

Same diff collection, but your text is appended to the tool call as an instruction the agent honors when it writes the summary. Note the difference: the argument steers what the agent emphasizes when it describes the changes, it does not add a reviewer or change any model. To change who reviews, edit the slot.

The bare form also has one behavior the argument form drops, the empty-diff check. With an instruction the agent calls the tool regardless, which is what you want when you are pointing at something specific.

You can also just ask for a multi-perspective review. The agent picks the `multi_review` tool on its own.

## What you get back

One markdown section per reviewer, separated by rules, then the main agent's consolidation.

A reviewer whose model is not selectable is listed under "Skipped reviewers" with the reason, rather than being silently dropped. That happens when `allowed_models` or `excluded_models` filters the spec out, or when you are not logged in to that provider.

A reviewer that fails gets a `FAILED` section naming the cause, plus a desktop notification. The run still returns the reviewers that worked. The tool reports an error only when nothing could run or every reviewer failed.
## Development

`nix develop` gives you everything the checks need: Rust 1.95 with rust-analyzer, `cargo-nextest`, `just`, `stylua`, and `nixfmt`. The toolchain matches the one maki's own flake pins, and the shell sets `OPENSSL_NO_VENDOR=1` so `openssl-sys` links the shell's OpenSSL instead of building its own.

```text
just check    # cargo check --tests
just lint     # cargo clippy --tests -- -D warnings
just test     # cargo nextest run
just fmt-lua  # stylua plugin/
```

The plugin is Lua, so Rust is here only to build the maki host. The tests load `plugin/` through the real `PluginHost`, which means a syntax error or a bad registration fails `just test`.
