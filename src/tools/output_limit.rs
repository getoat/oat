use rig::{
    tool::{ToolDyn, ToolError},
    wasm_compat::WasmBoxedFuture,
};

use crate::tool_policy::ToolOutputPolicy;

pub(crate) struct OutputLimitedTool {
    inner: Box<dyn ToolDyn>,
    output: ToolOutputPolicy,
}

impl OutputLimitedTool {
    pub(crate) fn new(inner: Box<dyn ToolDyn>, output: ToolOutputPolicy) -> Self {
        Self { inner, output }
    }
}

impl ToolDyn for OutputLimitedTool {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn definition<'a>(
        &'a self,
        prompt: String,
    ) -> WasmBoxedFuture<'a, rig::completion::ToolDefinition> {
        self.inner.definition(prompt)
    }

    fn call<'a>(&'a self, args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        Box::pin(async move {
            let output = self.inner.call(args).await?;
            let Ok(raw_output) = serde_json::from_str::<String>(&output) else {
                return Ok(output);
            };
            serde_json::to_string(&self.output.truncate(&self.name(), &raw_output))
                .map_err(ToolError::JsonError)
        })
    }
}
