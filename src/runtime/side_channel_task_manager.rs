use std::collections::HashMap;

use tokio::task::JoinHandle;

#[derive(Default)]
pub(crate) struct SideChannelTaskManager {
    active_tasks: HashMap<u64, JoinHandle<()>>,
}

impl SideChannelTaskManager {
    pub(crate) fn clear_completed_task(&mut self, reply_id: u64) {
        self.active_tasks.remove(&reply_id);
    }

    pub(crate) fn spawn_task(&mut self, reply_id: u64, task: JoinHandle<()>) {
        self.active_tasks.insert(reply_id, task);
    }

    pub(crate) fn cancel_all(&mut self) {
        for (_, task) in self.active_tasks.drain() {
            task.abort();
        }
    }
}
