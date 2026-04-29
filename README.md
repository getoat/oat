# Oat

Oat is a terminal-first AI coding agent written in Rust. It provides an
interactive TUI for day-to-day development work, plus headless modes for
scripted prompts and plan-driven execution.

## Features

- Interactive terminal UI with transcript history, model selection, approvals,
  and tool output controls.
- Headless execution with optional planning support:
  `oat --headless "prompt"` or `oat --headless-plan "prompt"`.
- Provider support through the model registry, including Azure OpenAI, Codex,
  OpenRouter, OpenCode Go, Ollama Cloud, and Chutes.
- Local tools for file operations, shell commands, web lookup, memory, todos,
  background terminals, and subagents.
- Configurable safety model, memory model, planning agents, approval mode, and
  tool policy.

## Getting Started

Build and run the TUI:

```sh
cargo run -- --config config.toml
```

Run a one-shot prompt:

```sh
cargo run -- --headless "summarize this repo"
```

Run a planning flow:

```sh
cargo run -- --headless-plan --planning-agent gpt-5.4::high "implement the plan"
```

Copy `config.example.toml` to `config.toml` and fill in the provider sections
for the models you want to use.

## Development

Before finishing Rust changes, run:

```sh
cargo fmt --check
cargo test
```
