local function reviewer_name(reviewer, index)
  if reviewer.name then
    return reviewer.name
  end
  local model_name = reviewer.model_name
  if model_name then
    local slash = model_name:find("/[^/]*$")
    local suffix = slash and model_name:sub(slash + 1) or model_name
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
  if reviewer.model_name then
    input.model = reviewer.model_name
  end
  if reviewer.thinking then
    input.thinking = reviewer.thinking
  end
  if reviewer.model_tier then
    input.model_tier = reviewer.model_tier
  end

  local value, err = maki.agent.call_tool(ctx, "task", input)
  if value == nil or value == "" then
    error(err or "returned nothing", 0)
  end
  return value
end

local reviewers_slot = maki.api.declare_slot("multireview.reviewers", function()
  return {}
end)

local function is_optional_string(v)
  return v == nil or type(v) == "string"
end

local function effective_reviewers()
  local list = reviewers_slot()
  if type(list) ~= "table" or #list == 0 then
    if list ~= nil then
      maki.log.warn("multi-review: multireview.reviewers slot returned no usable list.")
    end
    return nil
  end
  for i, reviewer in ipairs(list) do
    if
      type(reviewer) ~= "table"
      or not is_optional_string(reviewer.name)
      or not is_optional_string(reviewer.model_name)
      or not (is_optional_string(reviewer.thinking) or type(reviewer.thinking) == "number")
      or not is_optional_string(reviewer.model_tier)
    then
      return nil,
        ("reviewer %d takes optional string name, model_name, model_tier, and string or number thinking."):format(i)
    end
  end
  return list
end

local CONFIG_HINT = [[No reviewer is configured. Set the list in the global init.lua:

  maki.api.set_slot("multireview.reviewers", function()
    return { { model_name = "deepseek/deepseek-flash" }, { model_name = "claude/claude-opus-5" } }
  end)]]

local function failure_cause(r)
  if r.ok then
    return "returned nothing"
  end
  local err = tostring(r.err or "unknown error")
  err = err:gsub("^runtime error: ", ""):gsub("\nstack traceback:.*$", "")
  return err
end

local function task_rejects_model(ctx)
  local ok, defs = pcall(maki.agent.tools, ctx, { audience = "main" })
  if not ok or type(defs) ~= "table" then
    return false
  end
  for _, def in ipairs(defs) do
    if type(def) == "table" and def.name == "task" then
      local properties = def.input_schema and def.input_schema.properties
      if type(properties) ~= "table" then
        return false
      end
      return properties.model == nil
    end
  end
  return false
end

local function downgraded_reviewers(list, session, options)
  if type(session) ~= "table" or session.supports_thinking == false then
    return {}
  end
  local ceiling = session.thinking
  local by_name = {}
  if type(options) == "table" then
    for _, option in ipairs(options) do
      if type(option) == "table" and option.name then
        by_name[option.name] = option.tokens
      end
    end
  end
  local out = {}
  for i, reviewer in ipairs(list) do
    local asked = reviewer.thinking
    if asked ~= nil and asked ~= ceiling then
      local above
      if ceiling == "off" then
        above = true
      elseif by_name[asked] and by_name[ceiling] then
        if asked == "adaptive" or ceiling == "adaptive" then
          above = ceiling ~= "adaptive"
        else
          above = by_name[asked] > by_name[ceiling]
        end
      end
      if above then
        out[#out + 1] = { name = reviewer_name(reviewer, i), asked = tostring(asked), ran = tostring(ceiling) }
      end
    end
  end
  return out
end

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

    local list, config_error = effective_reviewers()
    if not list then
      maki.log.error("multi-review: " .. (config_error or "multireview.reviewers slot returned no usable list"))
      return {
        llm_output = config_error and ("Reviewer configuration is wrong: " .. config_error) or CONFIG_HINT,
        format = "markdown",
        is_error = true,
      }
    end

    local available = maki.model.available()
    local allowed
    if available then
      allowed = {}
      for _, spec in ipairs(available) do
        allowed[spec] = true
      end
    end

    if task_rejects_model(ctx) then
      local named = {}
      for i, reviewer in ipairs(list) do
        if reviewer.model_name then
          named[#named + 1] = reviewer_name(reviewer, i)
        end
      end
      if #named > 0 then
        return {
          llm_output = (
            "These reviewers pin a model_name, but the task tool does not accept one: %s\n\n"
            .. "Enable it in your config, then try again:\n\n  maki.setup({\n    plugins = {\n      task = { allow_model = true },\n    },\n  })\n\n"
            .. "Without it every reviewer runs on the session model."
          ):format(table.concat(named, ", ")),
          format = "markdown",
          is_error = true,
        }
      end
    end

    local fns = {}
    local subjects = {}
    local skipped = {}
    local notes = {}

    local session_ok, session = pcall(maki.model.get)
    if session_ok and type(session) == "table" then
      local downgraded = downgraded_reviewers(list, session, session.thinking_options)
      if #downgraded > 0 then
        local lines = {}
        for _, entry in ipairs(downgraded) do
          lines[#lines + 1] = ("%s asked for %s, ran at %s"):format(entry.name, entry.asked, entry.ran)
        end
        notes[#notes + 1] = ("Thinking is capped at this session (%s), so these were lowered:\n%s"):format(
          tostring(session.thinking),
          table.concat(lines, "\n")
        )
      end
    end
    for i, reviewer in ipairs(list) do
      if reviewer.model_name and allowed and not allowed[reviewer.model_name] then
        skipped[#skipped + 1] = ("%s (%s, not selectable: blocked by allowed_models/excluded_models or not logged in)"):format(
          reviewer_name(reviewer, i),
          reviewer.model_name
        )
      else
        subjects[#subjects + 1] = { reviewer, i }
        fns[#fns + 1] = function()
          return spawn_reviewer(ctx, reviewer, context, i)
        end
      end
    end

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
      local model_name = reviewer.name and reviewer.model_name
      local header = "## " .. reviewer_name(reviewer, index) .. (model_name and (" - " .. model_name) or "")
      if r.ok and r.value and r.value ~= "" then
        sections[#sections + 1] = header .. "\n\n" .. r.value
      else
        failed = failed + 1
        local cause = failure_cause(r)
        sections[#sections + 1] = header .. "\n\nFAILED: " .. cause
        maki.ui.flash("multi-review " .. reviewer_name(reviewer, index) .. " failed: " .. cause)
      end
    end

    if failed == #fns then
      notes[#notes + 1] = "Every reviewer that ran failed. Last error: " .. failure_cause(results[#results])
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
      target = [=[Run the multi_review tool. Gather the changes to be reviewed from the following instructions, then call multi_review with a summary of the files and what changed. Instructions: ]=]
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
