mod arch_context;
mod reschedule;
mod task;

use crate::paging;

pub(self) use arch_context::ArchContext;
pub use reschedule::{current_task, reschedule};
pub use task::{Pid, TaskControl, TaskDirectory, TaskReference, TASK_DIRECTORY};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SchedulerError {
    MemoryError(paging::MemoryError),
    OutOfPids,
}

impl From<paging::MemoryError> for SchedulerError {
    fn from(memory_error: paging::MemoryError) -> Self {
        Self::MemoryError(memory_error)
    }
}

pub type Result<T> = core::result::Result<T, SchedulerError>;

pub unsafe fn init(
    cpu_id: usize,
    _is_bsp: bool,
    idle_thread_stack: paging::KernelStack,
) -> Result<TaskReference> {
    let idle_task = task::Task::new_idle(cpu_id, idle_thread_stack)?;
    idle_task.clone().make_current();
    Ok(idle_task)
}

pub unsafe fn spawn(func: impl FnOnce() -> !) -> Result<TaskReference> {
    let ret = task::Task::spawn()?;

    let arch_context = {
        let mut arch_context = ArchContext::new();
        arch_context.set_stack(ret.stack_top());

        // TODOTODOTODO
        arch_context.set_page_table(x86::controlregs::cr3() as usize);
        arch_context.push_system_task_startup(func);

        arch_context
    };

    ret.clone().make_runnable(arch_context);
    Ok(ret)
}
