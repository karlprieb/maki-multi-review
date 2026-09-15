local function reviewer_name(reviewer, index)
  if reviewer.name then
    return reviewer.name
  end
  local model = reviewer.model
  if model then
    local slash = model:find("/[^/]*$")
    local suffix = slash and model:sub(slash + 1) or model
    return tostring(index) .. "-" .. suffix
  end
  return tostring(index)
end

local function spawn_reviewer(ctx, reviewer, context, index)
  local name = reviewer_name(reviewer, index)
  local prompt = ("Review these changes from a %s angle. Report only concrete findings, do not apply fixes. Number each finding in a numbered list as `1. {file path}:{line numbers} - short summary of the finding`. Use an em dash after the line range. Each finding is one numbered item and may span multiple lines. Output plain text, no preamble.\n\nChanges under review:\n%s"):format(
    name,
    context
  )

  local input = {
    description = "multi-review " .. name,
    prompt = prompt,
    subagent_type = "research",
  }
  if reviewer.model then
    input.model = reviewer.model
  end

  return maki.agent.call_tool(ctx, "task", input)
end

local reviewers_slot = maki.api.declare_slot("multireview.reviewers", function()
  return {}
end)

local function effective_reviewers()
  local list = reviewers_slot()
  if type(list) ~= "table" or #list == 0 then
    if list ~= nil then
      maki.log.warn("multi-review: multireview.reviewers slot returned no usable list.")
    end
    return nil
  end
  for i, reviewer in ipairs(list) do
    if type(reviewer) ~= "table" or type(reviewer.name or "") ~= "string" or type(reviewer.model or "") ~= "string" then
      maki.log.error(("multi-review: reviewer %d needs an optional string name and model."):format(i))
      return nil
    end
  end
  return list
end

local CONFIG_HINT = [[No reviewer is configured. Set the list in the global init.lua:

  maki.api.set_slot("multireview.reviewers", function()
    return { { model = "deepseek/deepseek-flash" }, { model = "claude/claude-opus-5" } }
  end)]]

maki.api.register_tool({
  name = "multi_review",
  kind = "execute",
  audiences = { "main" },
  description = [[Run a multi-perspective code review by spawning one read-only reviewer subagent per configured role, each with its own exact model, in parallel. Use when the user asks for a multi-perspective review. The agent should gather the changes (git diff) first and pass a summary in the `context` field. Reviewer roles and models come from the `multireview.reviewers` slot, which the global init.lua sets and a trusted project's .maki/init.lua can override. A reviewer whose model is blocked by allowed_models/excluded_models is reported as skipped rather than silently dropped. The tool returns the individual reviewers' findings; consolidate them yourself: deduplicate overlapping findings, drop anything speculative or out of scope, keep every concrete `file:line` finding, and group by theme.]],

  schema = {
    type = "object",
    properties = {
      context = {
        type = "string",
        description = "Overview of the changes under review (files touched and what changed).",
      },
    },
    required = { "context" },
  },

  handler = function(input, ctx)
    local context = input.context

    local list = effective_reviewers()
    if not list then
      return { llm_output = CONFIG_HINT, format = "markdown", is_error = true }
    end

    local available = maki.model.available()
    local allowed
    if available then
      allowed = {}
      for _, spec in ipairs(available) do
        allowed[spec] = true
      end
    end

    local fns = {}
    local subjects = {}
    local skipped = {}
    for i, reviewer in ipairs(list) do
      if reviewer.model and allowed and not allowed[reviewer.model] then
        skipped[#skipped + 1] = ("%s (%s, not selectable: blocked by allowed_models/excluded_models or not logged in)"):format(
          reviewer_name(reviewer, i),
          reviewer.model
        )
      else
        subjects[#subjects + 1] = { reviewer, i }
        fns[#fns + 1] = function()
          return spawn_reviewer(ctx, reviewer, context, i)
        end
      end
    end

    local notes = {}
    if #skipped > 0 then
      notes[#notes + 1] = "Skipped reviewers:\n" .. table.concat(skipped, "\n")
    end

    if #fns == 0 then
      notes[#notes + 1] =
        "No reviewer could run. Add a selectable model to the reviewers list, or remove it from excluded_models."
      return { llm_output = table.concat(notes, "\n\n"), format = "markdown", is_error = true }
    end

    local results = maki.async.gather(fns)

    local sections = {}
    local failed = 0
    for i, r in ipairs(results) do
      local reviewer, index = subjects[i][1], subjects[i][2]
      local model = reviewer.name and reviewer.model
      local header = "## " .. reviewer_name(reviewer, index) .. (model and (" - " .. model) or "")
      if r.ok and r.value and r.value ~= "" then
        sections[#sections + 1] = header .. "\n\n" .. r.value
      else
        failed = failed + 1
        local cause = r.ok and "returned nothing" or (r.err or "unknown error")
        sections[#sections + 1] = header .. "\n\nFAILED: " .. cause
        maki.notify(
          "multi-review " .. reviewer_name(reviewer, index) .. " failed: " .. cause,
          "error",
          { title = "multi-review" }
        )
      end
    end

    if failed == #fns then
      notes[#notes + 1] = "Every reviewer that ran failed. Last error: " .. (results[#results].err or "unknown error")
      return { llm_output = table.concat(notes, "\n\n"), format = "markdown", is_error = true }
    end

    local body = table.concat(sections, "\n\n---\n\n")
    if #notes > 0 then
      body = table.concat(notes, "\n\n") .. "\n\n---\n\n" .. body
    end

    return { llm_output = body, format = "markdown" }
  end,
})

maki.api.register_command({
  name = "/multi-review",
  description = "Multi-perspective review via multi_review subagents.",
  nargs = "*",
  handler = function(opts)
    local args = opts.args
    local target
    if args and args ~= "" then
      target = [=[Run the multi_review tool. First gather the current changes with `git diff` (unstaged) and `git diff --cached` (staged), then call multi_review with a summary of the files and what changed from both, honoring this instruction: ]=]
        .. args
    else
      target =
        [[Run the multi_review tool. First gather the current changes with `git diff` (unstaged) and `git diff --cached` (staged). If both are empty (or the directory is not a git repo), stop and tell the user there is nothing to review instead of calling multi_review. Otherwise call multi_review with a summary of the files and what changed from both.]]
    end
    local _, err = maki.session.prompt(target)
    if err then
      maki.ui.flash("multi-review: " .. err)
    end
  end,
})
