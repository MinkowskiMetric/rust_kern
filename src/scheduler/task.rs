use super::arch_context::ArchContext;
use super::{reschedule, reschedule::set_initial_task, Result, SchedulerError};
use crate::paging;
use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::sync::Arc;
use bitflags::bitflags;
use core::cell::UnsafeCell;
use intrusive_collections::intrusive_adapter;
use intrusive_collections::{LinkedList, LinkedListLink};
use spin::{Mutex, RwLock};

bitflags! {
    pub struct TaskFlags: u64 {
        const NO_TERMINATE = 1 << 0;
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TaskState {
    New,
    Ready,
    Running,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
#[repr(usize)]
pub enum TaskPriority {
    Idle = 0,
    Normal = 1,
}

const PRIORITIES_COUNT: usize = 2;

pub type Pid = usize;

const MIN_SYSTEM_PID: Pid = 0xfff8_0000_0000_0000;
const MAX_PID: Pid = 0x0007_ffff_ffff_ffff;

struct TaskDirectoryData {
    process_map: BTreeMap<Pid, TaskReference>,
    ready_lists: [LinkedList<TaskListAdapter>; 2],
    next_pid: Pid,
    next_system_pid: Pid,
}

impl TaskDirectoryData {
    const fn new() -> Self {
        Self {
            process_map: BTreeMap::new(),
            ready_lists: [LinkedList::new(TaskListAdapter::NEW); PRIORITIES_COUNT],
            next_pid: 0,
            next_system_pid: 0xffff_ffff_ffff_ffff,
        }
    }

    fn generate_pid(&mut self, system_task: bool) -> Result<Pid> {
        Ok(if system_task {
            if self.next_system_pid <= MIN_SYSTEM_PID {
                self.next_system_pid = 0xffff_ffff_ffff_ffff;
            }

            while self.process_map.contains_key(&self.next_system_pid) {
                self.next_system_pid -= 1;
            }

            if self.next_system_pid <= MIN_SYSTEM_PID {
                return Err(SchedulerError::OutOfPids);
            }

            let pid = self.next_system_pid;
            self.next_system_pid -= 1;
            pid
        } else {
            if self.next_pid >= MAX_PID {
                self.next_pid = 0;
            }

            while self.process_map.contains_key(&self.next_pid) {
                self.next_pid += 1;
            }

            if self.next_pid >= MAX_PID {
                return Err(SchedulerError::OutOfPids);
            }

            let pid = self.next_pid;
            self.next_pid += 1;
            pid
        })
    }

    fn create_task(&mut self, system_task: bool, init: TaskInit) -> Result<TaskReference> {
        let pid = self.generate_pid(system_task)?;

        let task = Arc::new(Task {
            pid,
            arch_context: ContextWrapper(UnsafeCell::new(ArchContext::new())),
            inner: RwLock::new(TaskData {
                _pid: pid,
                state: TaskState::New,
                init,
            }),
        });
        self.process_map.insert(pid, task.clone());
        Ok(task)
    }

    fn add_to_ready_list(&mut self, task_control: Box<TaskControl>) {
        let priority_index = {
            let task_inner = task_control.task.inner.read();
            assert_eq!(task_inner.state, TaskState::Ready);

            task_inner.init.priority as usize
        };

        self.ready_lists[priority_index].push_back(task_control);
    }

    fn find_next_task(
        &mut self,
        current_priority: Option<TaskPriority>,
    ) -> Option<Box<TaskControl>> {
        let min_priority_index = current_priority.map(|pri| pri as usize).unwrap_or(0);
        for priority_index in (min_priority_index..PRIORITIES_COUNT).rev() {
            let mut pos = self.ready_lists[priority_index].front_mut();
            while !pos.is_null() {
                let this_cpu = crate::cpu_id();
                let affinity_cpu = pos.get().unwrap().task().inner.read().init.cpu_id.unwrap_or(this_cpu);
                if this_cpu == affinity_cpu {
                    return Some(pos.remove().unwrap());
                } else {
                    pos.move_next();
                }
            }
        }

        // We didn't find a higher priority task
        None
    }
}

pub struct TaskDirectory {
    data: Mutex<TaskDirectoryData>,
}

impl TaskDirectory {
    const fn new() -> Self {
        Self {
            data: Mutex::new(TaskDirectoryData::new()),
        }
    }

    pub(super) fn create_task(
        &self,
        system_task: bool,
        task_data: TaskInit,
    ) -> Result<TaskReference> {
        self.data.lock().create_task(system_task, task_data)
    }

    pub(super) fn add_to_ready_list(&self, task_control: Box<TaskControl>) {
        self.data.lock().add_to_ready_list(task_control)
    }

    pub(super) fn find_next_task(
        &self,
        current_priority: Option<TaskPriority>,
    ) -> Option<Box<TaskControl>> {
        self.data.lock().find_next_task(current_priority)
    }
}

pub static TASK_DIRECTORY: TaskDirectory = TaskDirectory::new();

pub struct TaskInit {
    _flags: TaskFlags,
    kernel_stack: paging::KernelStack,
    cpu_id: Option<usize>,
    priority: TaskPriority,
}

pub struct TaskData {
    _pid: Pid,
    state: TaskState,
    init: TaskInit,
}

pub struct TaskControl {
    task: TaskReference,
    link: LinkedListLink,
    arch_context: ArchContext,
}

intrusive_adapter!(TaskListAdapter = Box<TaskControl>: TaskControl { link: LinkedListLink });

impl TaskControl {
    pub fn task(&self) -> TaskReference {
        self.task.clone()
    }

    pub fn arch_context<'a>(&'a mut self) -> &'a mut ArchContext {
        &mut self.arch_context
    }

    pub fn make_ready(self: Box<Self>) {
        {
            let mut lock = self.task.inner.write();

            // This can only happen for tasks in the running state
            assert_eq!(lock.state, TaskState::Running);
            lock.state = TaskState::Ready;
        }

        TASK_DIRECTORY.add_to_ready_list(self);
    }
}

struct ContextWrapper(UnsafeCell<ArchContext>);

unsafe impl Send for ContextWrapper {}
unsafe impl Sync for ContextWrapper {}

pub struct Task {
    pid: Pid,
    inner: RwLock<TaskData>,
    arch_context: ContextWrapper,
}

pub type TaskReference = Arc<Task>;

impl Task {
    pub(super) fn new_idle(
        cpu_id: usize,
        kernel_stack: paging::KernelStack,
    ) -> Result<TaskReference> {
        TASK_DIRECTORY.create_task(
            true,
            TaskInit {
                _flags: TaskFlags::NO_TERMINATE,
                kernel_stack: kernel_stack,
                cpu_id: Some(cpu_id),
                priority: TaskPriority::Idle,
            },
        )
    }

    pub(super) fn spawn() -> Result<TaskReference> {
        let kernel_stack = paging::allocate_kernel_stack(paging::DEFAULT_KERNEL_STACK_PAGES)?;

        TASK_DIRECTORY.create_task(
            false,
            TaskInit {
                _flags: TaskFlags::empty(),
                kernel_stack,
                cpu_id: None,
                priority: TaskPriority::Normal,
            },
        )
    }

    pub fn pid(&self) -> usize {
        self.pid
    }

    pub fn state(&self) -> TaskState {
        self.inner.read().state
    }

    pub fn set_running(&self) {
        let mut guard = self.inner.write();
        assert!(guard.state == TaskState::Ready);
        guard.state = TaskState::Running;
    }

    pub fn priority(&self) -> TaskPriority {
        self.inner.read().init.priority
    }

    pub fn stack_top(&self) -> usize {
        self.inner.read().init.kernel_stack.stack_top()
    }

    pub unsafe fn arch_context_ptr(&self) -> *mut ArchContext {
        self.arch_context.0.get()
    }

    pub(super) unsafe fn make_current(self: TaskReference) {
        // We don't need to set up anything in particular for the idle thread arch context
        // because it is already running
        let control = box TaskControl {
            task: self,
            link: LinkedListLink::new(),
            arch_context: ArchContext::new(),
        };

        {
            let mut lock = control.task.inner.write();

            // This can only happen for tasks in the new state
            assert_eq!(lock.state, TaskState::New);
            lock.state = TaskState::Running;
        }

        set_initial_task(control);
    }

    pub(super) unsafe fn make_runnable(self: TaskReference, arch_context: ArchContext) {
        let control = box TaskControl {
            task: self,
            link: LinkedListLink::new(),
            arch_context,
        };

        {
            let mut lock = control.task.inner.write();

            // This can only happen for tasks in the new state
            assert_eq!(lock.state, TaskState::New);
            lock.state = TaskState::Ready;
        };

        TASK_DIRECTORY.add_to_ready_list(control);
        reschedule();
    }
}
