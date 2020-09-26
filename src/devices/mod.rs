use crate::init::AP_READY;
use crate::paging::{self, PAGE_SIZE};
use crate::physmem::Frame;
use core::sync::atomic::Ordering;

pub mod io_apic;
pub mod local_apic;

pub unsafe fn init_bsp() {
    local_apic::init_bsp();
    io_apic::init();
}

pub unsafe fn init_ap(_cpu_id: usize) {
    local_apic::init_ap();
}

const TRAMPOLINE_P4: usize = 0x7000;
const TRAMPOLINE: usize = 0x8000;
static TRAMPOLINE_DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/trampoline"));

#[derive(Debug)]
struct ApStartupData {
    kernel_stack: paging::KernelStack,
    cpu_id: usize,
    cr3: usize,
}

pub unsafe fn start_aps() {
    let mut acpi_lock = crate::acpi::ACPI.lock();
    let acpi = acpi_lock.as_mut().unwrap();

    // First thing we have to do is to identity map the trampoline. We do this because
    // when the trampoline enables paging, it needs to be able to continue running
    {
        let mut page_table = paging::lock_page_table();
        let flush = page_table
            .map_to(
                TRAMPOLINE,
                Frame::containing_address(TRAMPOLINE),
                paging::PresentPageFlags::WRITABLE,
            )
            .expect("Failed to map trampoline");
        flush.flush(&page_table);
    }

    let mut mapping = paging::map_physical_memory(
        TRAMPOLINE_P4,
        2 * PAGE_SIZE,
        paging::PhysicalMappingFlags::empty(),
    )
    .expect("Failed to map trampoline");
    let trampoline_p4 =
        core::slice::from_raw_parts_mut(mapping.as_mut_ptr_offset::<u64>(0), PAGE_SIZE / 8);
    let trampoline =
        core::slice::from_raw_parts_mut(mapping.as_mut_ptr_offset::<u8>(PAGE_SIZE), PAGE_SIZE);

    let kernel_page_table = paging::phys_to_virt_addr(x86::controlregs::cr3() as usize, PAGE_SIZE);
    let page_table = core::slice::from_raw_parts(kernel_page_table as *const u64, PAGE_SIZE / 8);
    for i in 0..512 {
        core::intrinsics::atomic_store(&mut trampoline_p4[i] as *mut _, page_table[i]);
    }

    // Copy the trampoline into the memory block we use for it
    for i in 0..TRAMPOLINE_DATA.len() {
        core::intrinsics::atomic_store(&mut trampoline[i] as *mut _, TRAMPOLINE_DATA[i]);
    }

    for ap in acpi.acpi_context.application_processors.iter() {
        if ap.state != acpi::ProcessorState::WaitingForSipi {
            continue;
        }

        assert_ne!(
            u32::from(ap.local_apic_id),
            local_apic::local_apic_access().id(),
            "BSP listed in ASP list"
        );

        let (startup_data, stack) = {
            let kernel_stack = paging::allocate_kernel_stack(paging::DEFAULT_KERNEL_STACK_PAGES)
                .expect("Failed to allocate kernel stack for AP");
            let cr3 = x86::controlregs::cr3() as usize;
            let stack = kernel_stack.stack_top();
            let cpu_id = ap.local_apic_id.into();
            let startup_data = box ApStartupData {
                kernel_stack,
                cpu_id,
                cr3,
            };

            (alloc::boxed::Box::into_raw(startup_data), stack)
        };

        crate::println!("Starting AP: {:?}", ap);

        let ap_ready = trampoline.as_ptr().offset(8) as *mut u64;
        let ap_stack = ap_ready.offset(1);
        let ap_startup_data = ap_ready.offset(2);
        let ap_code = ap_ready.offset(3);
        AP_READY.store(false, Ordering::SeqCst);

        use core::intrinsics::{atomic_load, atomic_store};
        atomic_store(ap_ready, 0);
        atomic_store(ap_stack, stack as u64);
        atomic_store(ap_startup_data, startup_data as u64);
        atomic_store(ap_code, enter_ap as u64);

        {
            let mut icr = 0x4500;
            icr |= (ap.local_apic_id as u64) << 56;

            crate::println!("Sending init IPI");
            local_apic::local_apic_access().set_icr(icr);
        }

        {
            let ap_segment = (TRAMPOLINE >> 12) & 0xFF;
            let mut icr = 0x4600 | ap_segment as u64;

            icr |= (ap.local_apic_id as u64) << 56;

            crate::println!("Sending start IPI");
            local_apic::local_apic_access().set_icr(icr);
        }

        // Wait for trampoline ready
        crate::println!("Waiting for trampoline ready signal");
        while atomic_load(ap_ready) == 0 {
            crate::interrupts::pause();
        }

        crate::println!("Waiting for processor startup");
        while !AP_READY.load(Ordering::SeqCst) {
            crate::interrupts::pause();
        }

        crate::println!("AP started");
    }
}

unsafe extern "C" fn enter_ap(startup_data: *mut ApStartupData) -> ! {
    let startup_data = *alloc::boxed::Box::from_raw(startup_data);

    // We set the page table to match the boot processor because we can
    x86::controlregs::cr3_write(startup_data.cr3 as u64);

    crate::init::kstart_ap(startup_data.cpu_id, startup_data.kernel_stack)
}
