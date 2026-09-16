# maki-multi-review

A [maki](https://github.com/tontinton/maki) package that reviews your working diff from several angles at once. It spawns one read-only subagent per configured reviewer, in parallel, each pinned to its own model. Every reviewer reads the same summary of the changes and reports findings as `file:line - summary`. The main agent then merges the reports, drops duplicates, and groups what is left by theme.

Two reviewers on the same model with different angles is a real use. So is one cheap model and one expensive one on the same diff, to see what the cheap one misses.

## Install

Add the package to your global `~/.config/maki/init.lua`:

```lua
maki.pack.add({ "https://github.com/karlprieb/maki-multi-review" })
```

Maki asks once before installing, records the commit in `pack-lock.json`, and loads the package at startup. The plugin requests no permissions, so there is no second prompt.

This package needs maki 0.5.4 or newer. `plugin.toml` declares that floor, and an older maki skips the plugin rather than loading it half working. The floor exists because `thinking` and `model_tier` reach the reviewer through the `task` tool, which only accepts them from 0.5.4 on.

To pin a release, pass a table instead of a bare string:

```lua
maki.pack.add({
  { src = "https://github.com/karlprieb/maki-multi-review", version = "v0.1.0" },
})
```

Run `/packupdate maki-multi-review` to move to a newer commit.

## Configure

No reviewer is configured by default, so the tool errors until you set a list. The list lives in the `multireview.reviewers` slot.

### Turn on the task model input

To use `model_name`, set the task plugin to allow a model:

```lua
maki.setup({
  plugins = {
    task = { allow_model = true },
  },
})
```

This is off by default because the field costs tokens in every task schema. With it off the `task` tool drops the model, so every reviewer would run on your session's model. The plugin checks for this and stops with an error naming the reviewers that pinned a model, rather than running them on the wrong one. `model_tier` and `thinking` need no such setup.

### Global

`~/.config/maki/init.lua` holds the list every project uses:

```lua
maki.api.set_slot("multireview.reviewers", function()
  return {
    { model_name = "deepseek/deepseek-flash" },
    { name = "security", model_name = "claude/claude-opus-5", thinking = "xhigh" },
    { name = "performance", model_tier = "weak" },
  }
end)
```

### Per project

A trusted project's `.maki/init.lua` sets the same slot and wins, because project config loads after global config and the newest layer runs first. Returning a list without calling `prev` replaces the global one:

```lua
maki.api.set_slot("multireview.reviewers", function()
  return {
    { name = "concurrency", model_name = "claude/claude-opus-5" },
    { name = "api compatibility", model_name = "claude/claude-opus-5" },
  }
end)
```

To keep the global reviewers and add to them, call `prev` and append:

```lua
maki.api.set_slot("multireview.reviewers", function(prev)
  local list = prev()
  table.insert(list, { name = "sql injection", model_name = "claude/claude-opus-5" })
  return list
end)
```

The slot is read on every invocation, so `/reload` is enough after an edit. No restart.

### Reviewer options

An entry is a table. Every field is optional, and an empty table `{}` is a valid reviewer that runs the current model with no angle.

| Field          | Type               | Default                       | What it does                                                                                                                                 |
| -------------- | ------------------ | ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`         | string             | index plus model suffix       | The review angle. It goes into the reviewer's prompt ("Review these changes from a security angle") and becomes its section header.          |
| `model_name`   | string             | the session's current model   | An exact model spec, the same string `/model` shows, such as `claude/claude-opus-5`. Needs `task.allow_model`.                               |
| `thinking`     | string or number   | the session's level           | Reasoning depth for this reviewer: `off`, `adaptive`, an effort level from `minimal` to `max`, or a token budget. Capped at the session's.   |
| `model_tier`   | string             | the session's model           | `strong`, `medium`, or `weak`. Picks a model by tier instead of naming a spec, which needs no login check.                                   |



`thinking` and `model_tier` are requests, not commands. The host caps both at your own session: a reviewer can ask for less depth or a weaker model, never more. That makes them safe to set in a config file. Keeping one reviewer at `xhigh` and the rest at `off` is the usual reason to reach for it.

The cap is silent, so a reviewer asking for more than the session runs is lowered to the session's level with no error. The report says which ones, and what they ran at:

```text
Thinking is capped at this session (medium), so these were lowered:
security asked for high, ran at medium
```

A session set to `adaptive` has no ceiling, so reviewers get what they ask for. A session set to `off` can run nothing above it, so every reviewer runs with thinking off. Raise your own level with `/thinking` if you want a reviewer to go deeper.

`model_name` and `model_tier` both pick the model, and `model_name` wins when you set both. Use `model_name` when you want one exact model, `model_tier` when you only care how strong it is.

The old `model` key is now an error rather than a silently ignored field. Replace it with `model_name`; the error names the reviewer and the replacement.

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

A reviewer that fails gets a `FAILED` section naming the cause, plus a flash in the status line. The run still returns the reviewers that worked. The tool reports an error only when nothing could run or every reviewer failed.
## Development

`nix develop` gives you everything the checks need: Rust 1.95 with rust-analyzer, `cargo-nextest`, `just`, `stylua`, and `nixfmt`. The toolchain matches the one maki's own flake pins, and the shell sets `OPENSSL_NO_VENDOR=1` so `openssl-sys` links the shell's OpenSSL instead of building its own.

```text
just check     # cargo check --tests
just lint      # cargo clippy --tests -- -D warnings
just test      # cargo nextest run
just fmt       # cargo fmt --all
just fmt-lua   # stylua plugin/
just fmt-check # both formatters in check mode, the way CI runs them
```

CI runs those same checks, so run `just fmt-check` before pushing. Rust formatting is the easy one to miss, since it is the only check with no failing test to point at it.

The plugin is Lua, so Rust is here only to build the maki host. The tests load `plugin/` through the real `PluginHost`, which means a syntax error or a bad registration fails `just test`.
