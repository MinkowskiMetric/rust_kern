use super::arch_context::ArchContext;
use super::{TaskControl, TaskReference, TASK_DIRECTORY};
use alloc::boxed::Box;

struct CurrentTask {
    current: Option<Box<TaskControl>>,
    old: Option<Box<TaskControl>>,
}

impl CurrentTask {
    pub const fn new() -> Self {
        Self {
            current: None,
            old: None,
        }
    }

    unsafe fn switch_running_task(
        &mut self,
        new_task: Box<TaskControl>,
    ) -> Option<Box<TaskControl>> {
        self.current.replace(new_task)
    }

    pub fn current_task(&self) -> TaskReference {
        self.current.as_ref().unwrap().task()
    }

    unsafe fn prepare_task_switch<'a>(
        &'a mut self,
        next: Box<TaskControl>,
    ) -> (&'a mut ArchContext, &'a mut ArchContext) {
        assert!(self.old.is_none(), "Task switch already in progress");

        // Shuffle the current task into the old slot, and move the new task in.
        self.old = self.current.replace(next);

        // At this point we can mark the new task as running. Both tasks are currently shown
        // as running, which is true in the sense that they are both owned by this CPU. The old
        // task will be dealt with once we have completed the context switch
        self.current.as_mut().unwrap().task().set_running();

        (
            self.old.as_mut().unwrap().arch_context(),
            self.current.as_mut().unwrap().arch_context(),
        )
    }

    unsafe fn complete_task_switch(&mut self) {
        assert!(!self.old.is_none(), "Task switch is not in progress");

        let old_task = self.old.take().unwrap();
        old_task.make_ready()
    }

    pub unsafe fn reschedule(&mut self) {
        // Reschedule is called at opportune times to reschedule tasks, but the current task continues to be
        // runnable. You should not be holding any kernel locks when you call this (i.e. running at passive level
        // should we get as far as that)
        if let Some(next_task) = TASK_DIRECTORY.find_next_task(Some(current_task().priority())) {
            // Now we can get the pointer to the outgoing task and the incoming task arch contexts.

            // Pulling off this task switch is tricky. Problems - firstly, there is no way to do this atomically
            // because we cannot possible hold any locks while we're doing it. Context switching would be easier
            // if we could ensure that new threads just started here, but they don't they "return" to somewhere
            // else. The big problem is that if you put the new task onto the ready list (or indeed the wait list)
            // there is a danger that another core will pick it up and run with it, and we can't hold any locks.

            // Redox solves this by serializing all context switches. So does NT. Basically all of this happens
            // inside "the dispatcher lock" which is the only lock you can hold over a context switch.
            // This gives us access to the outgoing process object, and removes it from the "current"
            // once we remove it, we must complete a task switch
            let (old_ctxt, new_ctxt) = CURRENT_TASK.prepare_task_switch(next_task);

            old_ctxt.switch_to(new_ctxt);

            todo!()
        } // otherwise, nothing currently ready to switch to so stay where we are
    }
}

pub fn current_task() -> TaskReference {
    unsafe { CURRENT_TASK.current_task() }
}

pub(super) unsafe fn set_initial_task(task_control: Box<TaskControl>) {
    assert!(CURRENT_TASK.switch_running_task(task_control).is_none());
}

#[thread_local]
static mut CURRENT_TASK: CurrentTask = CurrentTask::new();

pub fn reschedule() {
    unsafe {
        CURRENT_TASK.reschedule();
    }
}

#[no_mangle]
unsafe extern "C" fn complete_task_switch() {
    CURRENT_TASK.complete_task_switch()
}
