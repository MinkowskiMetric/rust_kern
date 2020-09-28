use crate::acpi;
use crate::allocator;
use crate::devices;
use crate::gdt;
use crate::idt;
use crate::paging;
use crate::physmem;
use crate::println;
use crate::scheduler;
use alloc::vec::Vec;
use bootloader::{bootinfo::MemoryRegion, BootInfo};
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub static AP_READY: AtomicBool = AtomicBool::new(false);
static BSP_READY: AtomicBool = AtomicBool::new(false);

#[thread_local]
static CPU_ID: AtomicUsize = AtomicUsize::new(0);

pub fn cpu_id() -> usize {
    CPU_ID.load(Ordering::SeqCst)
}

pub unsafe fn kstart(boot_info: &'static BootInfo, func: impl FnOnce() -> ! + 'static) -> ! {
    paging::pre_init(boot_info);

    println!("Starting kernel...");

    gdt::init();
    idt::early_init();

    physmem::early_init(boot_info.memory_map.iter());

    // Initialize the allocator before paging. The allocator uses a small internal buffer which
    // gives us enough working heap to allocate during paging initialization
    allocator::init();

    // Now that we have a functioning heap, we can make a copy of the boot memory map.
    // Eventually we will pass this to the paging manager instead of the one from the bootloader
    let memory_map: Vec<_> = boot_info.memory_map.iter().cloned().collect();

    let tcb_offset = paging::init(0);

    physmem::init_post_paging(memory_map.iter());

    // Once paging is up and running, we can allocate a new kernel stack
    // for what will become our idle thread
    let idle_thread_stack = paging::allocate_kernel_stack(paging::DEFAULT_KERNEL_STACK_PAGES)
        .expect("Failed to allocate first kernel stack");
    let fault_stack = paging::allocate_kernel_stack(paging::DEFAULT_KERNEL_STACK_PAGES)
        .expect("Failed to allocate fault stack");
    idle_thread_stack.switch_to_permanent(move |stack| {
        init_post_paging(stack, fault_stack, tcb_offset, memory_map, func);
    });
}

unsafe fn init_post_paging(
    idle_thread_stack: paging::KernelStack,
    fault_stack: paging::KernelStack,
    tcb_offset: usize,
    memory_map: Vec<MemoryRegion>,
    func: impl FnOnce() -> ! + 'static,
) -> ! {
    println!(
        "Running on our own stack! {:?} tcb: {:#x}",
        &idle_thread_stack as *const paging::KernelStack, tcb_offset,
    );

    gdt::init_post_paging(tcb_offset, &idle_thread_stack, &fault_stack);
    idt::init(true);

    CPU_ID.store(0, Ordering::SeqCst);

    // Once the GDT has got the fault stack, we don't need it any more. We keep the idle
    // thread stack because we need it for the idle task
    let _ = core::mem::ManuallyDrop::new(fault_stack);

    physmem::init_reclaim(memory_map.iter());

    acpi::init_bsp();

    // At this point, memory is fully working and in our control. The next thing to do is to bring up
    // the basic hardware
    devices::init_bsp();

    // Before starting the APs, create our idle task and initialize the schedule
    let idle_task =
        scheduler::init(0, true, idle_thread_stack).expect("Failed to create idle task for CPU 0");
    println!("idle task pid {}", idle_task.pid());

    // Once the devices are broadly set up, start the other proessors
    devices::start_aps();

    // Before we go into the idle loop ourselves, kick the aps
    BSP_READY.store(true, Ordering::SeqCst);

    // Spawn the init task
    {
        let init_task =
            scheduler::spawn(move || userland_init(func)).expect("Failed to spawn init task");
        println!("Spawned init task {}", init_task.pid());
    }

    crate::println!("CPU {} going idle", 0);

    idle_loop();
}

pub unsafe fn kstart_ap(cpu_id: usize, idle_thread_stack: paging::KernelStack) -> ! {
    println!("Starting AP {}", cpu_id);

    let tcb_offset = paging::init_ap(cpu_id);

    let fault_stack = paging::allocate_kernel_stack(paging::DEFAULT_KERNEL_STACK_PAGES)
        .expect("Failed to allocate AP fault stack");
    gdt::init_ap(tcb_offset, &idle_thread_stack, &fault_stack);
    idt::init(false);

    CPU_ID.store(cpu_id, Ordering::SeqCst);

    // Once the GDT has got the fault stack, we don't need it any more. We keep the idle
    // thread stack because we need it for the idle task
    let _ = core::mem::ManuallyDrop::new(fault_stack);

    devices::init_ap(cpu_id);

    // Create our idle task
    scheduler::init(cpu_id, false, idle_thread_stack).expect("Failed to create idle task for AP");

    // Finally, signal that we're done starting up
    AP_READY.store(true, Ordering::SeqCst);

    while !BSP_READY.load(Ordering::SeqCst) {
        crate::interrupts::pause();
    }

    crate::println!("CPU {} going idle", cpu_id);

    idle_loop()
}

fn userland_init(func: impl FnOnce() -> ! + 'static) -> ! {
    func()
}

pub fn idle_loop() -> ! {
    loop {
        unsafe {
            crate::interrupts::enable_and_halt();
        }
    }
}

/// This function is called on panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    use crate::ipi::{ipi, IpiKind, IpiTarget};
    ipi(IpiKind::Halt, IpiTarget::Other);
    crate::interrupts::disable_and_halt()
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::test_panic_handler(info)
}
