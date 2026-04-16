"""Harbor installed-agent wrapper for the local Oat CLI."""

from __future__ import annotations

import os
import re
import shlex
import shutil
import subprocess
from pathlib import Path

import toml
from harbor.agents.installed.base import BaseInstalledAgent, with_prompt_template
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext


class OatAgent(BaseInstalledAgent):
    """Run the local Oat CLI inside a Harbor task container."""

    _REPO_ROOT = Path(__file__).resolve().parents[2]
    _CONTAINER_BIN_PATH = "/usr/local/bin/oat"
    _CONTAINER_CONFIG_PATH = "/installed-agent/oat-config.toml"
    _CONTAINER_RUNNER_PATH = "/installed-agent/oat-runner.sh"
    _CONTAINER_CA_BUNDLE_PATH = "/installed-agent/ca-certificates.crt"
    _CONTAINER_LIB_DIR = "/installed-agent/lib"
    _CONTAINER_HOME = "/installed-agent/home"
    _OUTPUT_FILENAME = "oat.txt"
    _DEBUG_LOG_FILENAME = "oat-debug.log"
    _PASSTHROUGH_ENV_VARS = (
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "OPENAI_ORG_ID",
        "ANTHROPIC_API_KEY",
        "AZURE_OPENAI_API_KEY",
        "AZURE_OPENAI_ENDPOINT",
        "DEEPSEEK_API_KEY",
        "GOOGLE_API_KEY",
        "MISTRAL_API_KEY",
        "OPENROUTER_API_KEY",
        "CHUTES_API_KEY",
        "OLLAMA_API_KEY",
        "OPENCODE_API_KEY",
        "XAI_API_KEY",
    )
    _DEFAULT_CARGO_HOME = Path("/root/.cargo")
    _DEFAULT_RUSTUP_HOME = Path("/root/.rustup")
    _CA_BUNDLE_CANDIDATES = (
        Path("/etc/ssl/certs/ca-certificates.crt"),
        Path("/etc/pki/tls/certs/ca-bundle.crt"),
        Path("/etc/ssl/cert.pem"),
    )
    _COMPAT_RUNTIME_LIBRARY_NAMES = (
        "libssl.so.1.1",
        "libcrypto.so.1.1",
    )
    _DEFAULT_PLANNING_AGENT = {
        "model_name": "gpt-5.4-nano",
        "reasoning": "medium",
    }

    def __init__(
        self,
        logs_dir: Path,
        model_name: str | None = None,
        *,
        reasoning: str | None = None,
        headless_plan: bool = False,
        auto_accept_plan: bool = True,
        dangerous: bool = True,
        planning_agents: list[str] | str | None = None,
        debug_log: bool = False,
        oat_bin_path: str | None = None,
        config_path: str | None = None,
        build_profile: str = "debug",
        **kwargs,
    ):
        super().__init__(logs_dir=logs_dir, model_name=model_name, **kwargs)
        self._reasoning = reasoning
        self._headless_plan = headless_plan
        self._auto_accept_plan = auto_accept_plan
        self._dangerous = dangerous
        self._planning_agents = self._normalize_planning_agents(planning_agents)
        self._debug_log = debug_log
        self._oat_bin_path = Path(oat_bin_path).expanduser() if oat_bin_path else None
        self._config_path = Path(config_path).expanduser() if config_path else None
        self._build_profile = build_profile
        self._container_loader_path: str | None = None

    @staticmethod
    def name() -> str:
        return "oat"

    def version(self) -> str | None:
        return self._version or "local"

    def get_version_command(self) -> str | None:
        return f"{self._CONTAINER_BIN_PATH} --version"

    @staticmethod
    def _normalize_planning_agents(
        planning_agents: list[str] | str | None,
    ) -> list[str]:
        if planning_agents is None:
            return []
        if isinstance(planning_agents, str):
            return [
                entry.strip()
                for entry in planning_agents.split(",")
                if entry.strip()
            ]
        return [entry for entry in planning_agents if entry]

    def _resolve_host_binary(self) -> Path:
        if self._oat_bin_path is not None:
            if not self._oat_bin_path.exists():
                raise FileNotFoundError(
                    f"Oat binary not found at {self._oat_bin_path}"
                )
            return self._oat_bin_path

        compat_artifact = self._ensure_compat_binary()
        if compat_artifact is not None:
            return compat_artifact

        cargo_args = ["cargo", "build", "--bin", "oat"]
        if self._build_profile == "release":
            cargo_args.append("--release")
            artifact = self._REPO_ROOT / "target" / "release" / "oat"
        else:
            artifact = self._REPO_ROOT / "target" / "debug" / "oat"

        build_env = os.environ.copy()
        if "CARGO_HOME" not in build_env and self._DEFAULT_CARGO_HOME.exists():
            build_env["CARGO_HOME"] = str(self._DEFAULT_CARGO_HOME)
        if "RUSTUP_HOME" not in build_env and self._DEFAULT_RUSTUP_HOME.exists():
            build_env["RUSTUP_HOME"] = str(self._DEFAULT_RUSTUP_HOME)

        subprocess.run(
            cargo_args,
            cwd=self._REPO_ROOT,
            check=True,
            env=build_env,
        )
        if not artifact.exists():
            raise FileNotFoundError(f"Built Oat binary missing at {artifact}")
        return artifact

    @classmethod
    def _compat_artifact_path(cls) -> Path:
        return cls._REPO_ROOT / "target" / "compat-gnu" / "release" / "oat"

    @classmethod
    def _compat_runtime_dir(cls) -> Path:
        return cls._REPO_ROOT / "target" / "compat-gnu" / "runtime"

    @classmethod
    def _compat_binary_ready(cls) -> bool:
        compat_artifact = cls._compat_artifact_path()
        compat_runtime_dir = cls._compat_runtime_dir()
        return compat_artifact.exists() and all(
            (compat_runtime_dir / name).exists()
            for name in cls._COMPAT_RUNTIME_LIBRARY_NAMES
        )

    def _ensure_compat_binary(self) -> Path | None:
        compat_artifact = self._compat_artifact_path()
        if self._compat_binary_ready():
            return compat_artifact

        if shutil.which("docker") is None:
            return None

        build_script = Path(__file__).with_name("build_compat_oat.sh")
        if not build_script.exists():
            return None

        subprocess.run(
            ["/bin/sh", str(build_script)],
            cwd=self._REPO_ROOT,
            check=True,
        )
        if self._compat_binary_ready():
            return compat_artifact
        return None

    @staticmethod
    def _compat_runtime_dir_for_binary(host_binary: Path) -> Path | None:
        if "compat-gnu" not in host_binary.parts:
            return None
        runtime_dir = host_binary.parents[1] / "runtime"
        if runtime_dir.exists():
            return runtime_dir
        return None

    def _resolve_host_config(self) -> Path:
        if self._config_path is not None:
            if not self._config_path.exists():
                raise FileNotFoundError(
                    f"Oat config not found at {self._config_path}"
                )
            return self._config_path

        generated = self.logs_dir / "oat-harbor-config.toml"
        repo_config = self._REPO_ROOT / "config.toml"
        generated.write_text(
            self._benchmark_config_content(repo_config if repo_config.exists() else None),
            encoding="utf-8",
        )
        return generated

    def _benchmark_config_content(self, source_config: Path | None) -> str:
        source_data: dict[str, object] = {}
        if source_config is not None:
            source_data = toml.loads(source_config.read_text(encoding="utf-8"))

        source_model = source_data.get("model")
        if not isinstance(source_model, dict):
            source_model = {}

        model_name = self.model_name or source_model.get("model_name") or "gpt-5.4-mini"
        reasoning = self._reasoning or source_model.get("reasoning") or "medium"

        output: dict[str, object] = {
            "model": {
                "model_name": str(model_name),
                "reasoning": str(reasoning),
            },
            "safety": {
                "model_name": str(model_name),
                "reasoning": str(reasoning),
            },
        }

        required_provider_tables = self._required_provider_tables(
            str(model_name),
            [self._DEFAULT_PLANNING_AGENT["model_name"]],
        )
        for table_name in required_provider_tables:
            table = source_data.get(table_name)
            if isinstance(table, dict) and table:
                output[table_name] = table

        planning = source_data.get("planning")
        if not isinstance(planning, dict):
            planning = {}
        output["planning"] = {
            **planning,
            "agents": [dict(self._DEFAULT_PLANNING_AGENT)],
        }

        memory = source_data.get("memory")
        if not isinstance(memory, dict):
            memory = {}
        extraction = memory.get("extraction")
        if not isinstance(extraction, dict):
            extraction = {}
        output["memory"] = {
            **memory,
            "enabled": True,
            "extraction": {
                **extraction,
                "enabled": True,
                "model_name": str(model_name),
                "reasoning": str(reasoning),
            },
        }

        tools = source_data.get("tools")
        if not isinstance(tools, dict):
            tools = {}
        web_search = tools.get("web_search")
        if not isinstance(web_search, dict):
            web_search = {}
        output["tools"] = {
            **tools,
            "web_search": {**web_search, "mode": "live"},
        }
        return toml.dumps(output)

    @classmethod
    def _required_provider_tables(
        cls,
        main_model_name: str,
        planning_model_names: list[str],
    ) -> list[str]:
        tables: list[str] = []
        for model_name in [main_model_name, *planning_model_names]:
            table_name = cls._provider_table_for_model(model_name)
            if table_name and table_name not in tables:
                tables.append(table_name)
        return tables

    @staticmethod
    def _provider_table_for_model(model_name: str) -> str | None:
        if model_name.startswith("codex/"):
            return "codex"
        if model_name.startswith("openai/"):
            return "openrouter"
        if model_name.startswith("opencode-go/"):
            return "opencode"
        if model_name.startswith("gpt-") or model_name.startswith("kimi-"):
            return "azure"
        return None

    def _container_env(self) -> dict[str, str]:
        env = {"HOME": self._CONTAINER_HOME}
        env["LD_LIBRARY_PATH"] = self._CONTAINER_LIB_DIR
        if self._resolve_host_ca_bundle() is not None:
            env["SSL_CERT_FILE"] = self._CONTAINER_CA_BUNDLE_PATH
            env["CURL_CA_BUNDLE"] = self._CONTAINER_CA_BUNDLE_PATH
            env["REQUESTS_CA_BUNDLE"] = self._CONTAINER_CA_BUNDLE_PATH
        for key in self._PASSTHROUGH_ENV_VARS:
            value = os.environ.get(key)
            if value:
                env[key] = value
        if self._debug_log:
            env["OAT_DEBUG_LOG"] = "1"
            env["OAT_DEBUG_LOG_PATH"] = f"/logs/agent/{self._DEBUG_LOG_FILENAME}"
        return env

    @classmethod
    def _resolve_host_ca_bundle(cls) -> Path | None:
        for candidate in cls._CA_BUNDLE_CANDIDATES:
            if candidate.exists():
                return candidate
        return None

    @staticmethod
    def _resolve_host_runtime_bundle(host_binary: Path) -> tuple[Path | None, list[Path]]:
        output = subprocess.check_output(["ldd", str(host_binary)], text=True)
        absolute_paths: list[Path] = []
        for line in output.splitlines():
            match = re.search(r"=>\s+(/[^ ]+)", line)
            if match:
                absolute_paths.append(Path(match.group(1)))
                continue
            stripped = line.strip()
            if stripped.startswith("/"):
                absolute_paths.append(Path(stripped.split(" ", 1)[0]))

        loader: Path | None = None
        libraries: list[Path] = []
        seen: set[Path] = set()
        for path in absolute_paths:
            if path in seen or not path.exists():
                continue
            seen.add(path)
            if "ld-linux" in path.name:
                loader = path
            else:
                libraries.append(path)
        return loader, libraries

    async def install(self, environment: BaseEnvironment) -> None:
        host_binary = self._resolve_host_binary()
        host_config = self._resolve_host_config()
        host_runner = Path(__file__).with_name("oat_runner.sh")
        host_ca_bundle = self._resolve_host_ca_bundle()
        compat_runtime_dir = self._compat_runtime_dir_for_binary(host_binary)
        if compat_runtime_dir is not None:
            host_loader = None
            host_runtime_libraries = sorted(compat_runtime_dir.glob("*"))
        else:
            host_loader, host_runtime_libraries = self._resolve_host_runtime_bundle(host_binary)
        self._container_loader_path = None

        await self.exec_as_root(
            environment,
            command=f"mkdir -p {self._CONTAINER_HOME} {self._CONTAINER_LIB_DIR}",
        )
        await environment.upload_file(
            source_path=host_binary,
            target_path=self._CONTAINER_BIN_PATH,
        )
        await environment.upload_file(
            source_path=host_config,
            target_path=self._CONTAINER_CONFIG_PATH,
        )
        await environment.upload_file(
            source_path=host_runner,
            target_path=self._CONTAINER_RUNNER_PATH,
        )
        if host_ca_bundle is not None:
            await environment.upload_file(
                source_path=host_ca_bundle,
                target_path=self._CONTAINER_CA_BUNDLE_PATH,
            )
        for library_path in host_runtime_libraries:
            await environment.upload_file(
                source_path=library_path,
                target_path=f"{self._CONTAINER_LIB_DIR}/{library_path.name}",
            )
        if host_loader is not None:
            self._container_loader_path = f"{self._CONTAINER_LIB_DIR}/{host_loader.name}"
            await environment.upload_file(
                source_path=host_loader,
                target_path=self._container_loader_path,
            )
        await self.exec_as_root(
            environment,
            command=(
                f"chmod +x {self._CONTAINER_BIN_PATH} {self._CONTAINER_RUNNER_PATH}"
                + (
                    f" {self._container_loader_path}"
                    if self._container_loader_path is not None
                    else ""
                )
                + " && "
                f"chmod 600 {self._CONTAINER_CONFIG_PATH}"
            ),
        )

    def populate_context_post_run(self, context: AgentContext) -> None:
        output_path = self.logs_dir / self._OUTPUT_FILENAME
        if output_path.exists():
            context.metadata = {
                **(context.metadata or {}),
                "oat_output_path": str(output_path),
            }

    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        command_parts: list[str] = []
        if self._container_loader_path is not None:
            command_parts.extend(
                [
                    self._container_loader_path,
                    "--library-path",
                    self._CONTAINER_LIB_DIR,
                ]
            )

        command_parts.extend(
            [
                self._CONTAINER_BIN_PATH,
                "--config",
                self._CONTAINER_CONFIG_PATH,
            ]
        )

        if self._dangerous:
            command_parts.append("--dangerous")

        if self._headless_plan:
            command_parts.append("--headless-plan")
            if self._auto_accept_plan:
                command_parts.append("--auto-accept-plan")
        else:
            command_parts.append("--headless")

        if self.model_name:
            command_parts.extend(["--model", self.model_name])
        if self._reasoning:
            command_parts.extend(["--reasoning", self._reasoning])
        for planning_agent in self._planning_agents:
            command_parts.extend(["--planning-agent", planning_agent])

        command_parts.append("--")
        command_parts.append(instruction)

        quoted_command = " ".join(shlex.quote(part) for part in command_parts)
        run_command = " ".join(
            [
                "/bin/sh",
                shlex.quote(self._CONTAINER_RUNNER_PATH),
                "--log",
                shlex.quote(f"/logs/agent/{self._OUTPUT_FILENAME}"),
                "--stats-dir",
                shlex.quote(f"{self._CONTAINER_HOME}/.config/oat/stats"),
                "--",
                quoted_command,
            ]
        )
        await self.exec_as_agent(
            environment,
            command=run_command,
            env=self._container_env(),
        )
