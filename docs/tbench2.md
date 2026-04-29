# TBench2 / Harbor

Oat now has the minimum pieces needed to run under Harbor for Terminal-Bench 2 style tasks:

- Headless execution: `oat --headless "prompt"`
- Headless plan-only execution: `oat --headless-plan "prompt"`
- Headless plan + implementation: `oat --headless-plan --auto-accept-plan "prompt"`
- Runtime model overrides from the CLI:
  - `--model <model>`
  - `--reasoning <reasoning>`
  - repeatable `--planning-agent <model>::<reasoning>`
- Explicit config selection with `--config <path>`
- Write-enabled autonomous mode with `--dangerous`

## CLI examples

```bash
oat --headless "Fix the failing tests"
oat --headless --model gpt-5.4 --reasoning high "Implement the feature"
oat --headless-plan "Plan the migration"
oat --headless-plan --auto-accept-plan --dangerous "Plan and implement the refactor"
oat --headless-plan \
  --planning-agent gpt-5.4-mini::medium \
  --planning-agent kimi-k2.5::on \
  "Plan the rollout"
```

Notes:

- `--planning-agent` uses `<model>::<reasoning>` because some model names already contain `:`.
- `--auto-accept-plan` is only valid with `--headless-plan`.
- In dangerous mode, headless planning stays write-enabled end to end.
- In dangerous mode, Oat's path-based tools are no longer confined to the workspace root; they can operate against the full container filesystem.

## Harbor adapter

This repo now includes a Harbor installed-agent wrapper at `contrib/harbor/oat_agent.py`.

Example single-trial run:

```bash
PYTHONPATH=/root/oat uv run harbor trials start \
  -d terminal-bench/terminal-bench-2 \
  --agent-import-path contrib.harbor.oat_agent:OatAgent \
  -m openai/gpt-5.4-mini \
  --agent-kwargs headless_plan=true \
  --agent-kwargs auto_accept_plan=true
```

Useful Harbor kwargs for `OatAgent`:

- `reasoning="high"`
- `headless_plan=true`
- `auto_accept_plan=true`
- `dangerous=true`
- `planning_agents=["gpt-5.4-mini::medium","kimi-k2.5::on"]`
- `config_path="/abs/path/to/config.toml"`
- `oat_bin_path="/abs/path/to/oat"`
- `build_profile="release"`

Behavior:

- The Harbor adapter prefers a Bullseye-built compat binary at `target/compat-gnu/release/oat`.
- If that compat binary is missing and Docker is available, the adapter builds it automatically with `contrib/harbor/build_compat_oat.sh` and bundles Bullseye `libssl.so.1.1` / `libcrypto.so.1.1` into the task container. This avoids startup failures in older Terminal-Bench images.
- If the compat build path is unavailable, the adapter falls back to uploading a freshly built local Oat binary plus its discovered shared-library bundle.
- The adapter also uploads a small runner that watches Oat's stats directory and forcibly terminates the CLI if the headless session is finalized but the process does not exit on its own. This avoids Harbor `AgentTimeoutError` when Oat finishes the turn but leaves a stuck process behind.
- If the repo has a local `config.toml`, the adapter uploads that config by default so Harbor uses the same provider/model setup as the local app.
- If no repo `config.toml` exists, the adapter falls back to a generated config that enables memory, memory extraction, and live hosted web search for Harbor runs.
- If you pass `config_path`, that config is uploaded instead. This is the path to use if you need an explicit alternate provider setup or Codex auth material inside the task container.

## Out of scope

- ATIF export is not implemented for Oat's Harbor adapter in this pass.
- Terminal-Bench leaderboard submission flow is unchanged and handled by Harbor's existing output artifacts rather than a custom Oat submission path.
