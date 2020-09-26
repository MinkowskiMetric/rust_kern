use crate::paging::phys_to_virt_addr;
use acpi::{search_for_rsdp_bios, Acpi as AcpiContext, AcpiHandler, PhysicalMapping};
use aml::{AmlContext, DebugVerbosity, Handler as AmlHandler};
use core::marker::PhantomData;
use spin::Mutex;

pub struct HandlerImpl;

impl AcpiHandler for HandlerImpl {
    unsafe fn map_physical_region<T>(
        &mut self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<T> {
        let virtual_addr = phys_to_virt_addr(physical_address, size);

        PhysicalMapping {
            physical_start: physical_address,
            virtual_start: core::ptr::NonNull::new(virtual_addr as *mut T).unwrap(),
            region_length: size,
            mapped_length: size,
        }
    }

    fn unmap_physical_region<T>(&mut self, _region: PhysicalMapping<T>) {
        // Nothing to do here because we only support identity mapped memory
    }
}

impl AmlHandler for HandlerImpl {
    fn read_u8(&self, _address: usize) -> u8 {
        todo!()
    }
    fn read_u16(&self, _address: usize) -> u16 {
        todo!()
    }
    fn read_u32(&self, _address: usize) -> u32 {
        todo!()
    }
    fn read_u64(&self, _address: usize) -> u64 {
        todo!()
    }
    fn write_u8(&mut self, _address: usize, _value: u8) {
        todo!()
    }
    fn write_u16(&mut self, _address: usize, _value: u16) {
        todo!()
    }
    fn write_u32(&mut self, _address: usize, _value: u32) {
        todo!()
    }
    fn write_u64(&mut self, _address: usize, _value: u64) {
        todo!()
    }
    fn read_io_u8(&self, _port: u16) -> u8 {
        todo!()
    }
    fn read_io_u16(&self, _port: u16) -> u16 {
        todo!()
    }
    fn read_io_u32(&self, _port: u16) -> u32 {
        todo!()
    }
    fn write_io_u8(&self, _port: u16, _value: u8) {
        todo!()
    }
    fn write_io_u16(&self, _port: u16, _value: u16) {
        todo!()
    }
    fn write_io_u32(&self, _port: u16, _value: u32) {
        todo!()
    }
    fn read_pci_u8(&self, _segment: u16, _bus: u8, _device: u8, _function: u8, _offset: u16) -> u8 {
        todo!()
    }
    fn read_pci_u16(
        &self,
        _segment: u16,
        _bus: u8,
        _device: u8,
        _function: u8,
        _offset: u16,
    ) -> u16 {
        todo!()
    }
    fn read_pci_u32(
        &self,
        _segment: u16,
        _bus: u8,
        _device: u8,
        _function: u8,
        _offset: u16,
    ) -> u32 {
        todo!()
    }
    fn write_pci_u8(
        &self,
        _segment: u16,
        _bus: u8,
        _device: u8,
        _function: u8,
        _offset: u16,
        _value: u8,
    ) {
        todo!()
    }
    fn write_pci_u16(
        &self,
        _segment: u16,
        _bus: u8,
        _device: u8,
        _function: u8,
        _offset: u16,
        _value: u16,
    ) {
        todo!()
    }
    fn write_pci_u32(
        &self,
        _segment: u16,
        _bus: u8,
        _device: u8,
        _function: u8,
        _offset: u16,
        _value: u32,
    ) {
        todo!()
    }
}

pub struct Acpi<H: AmlHandler + AcpiHandler> {
    pub acpi_context: AcpiContext,
    pub aml_context: AmlContext,
    _marker: PhantomData<H>,
}

// AmlContext uses Box<dyn Handler> without requiring Sync and Send, which means that it is very
// difficult to put the handler in a mutex. Looking at the code the only reason it isn't send is
// because of the Box<dyn Handler> which should be Box<dyn Handler + Send>.
unsafe impl<H: AmlHandler + AcpiHandler + Send> Send for Acpi<H> {}

impl<H: 'static + AmlHandler + AcpiHandler> Acpi<H> {
    pub unsafe fn new(handler: H) -> Self {
        let mut handler = box handler;

        let acpi_context = search_for_rsdp_bios(handler.as_mut()).expect("ACPI RDSP not found");
        let mut aml_context = AmlContext::new(handler, false, DebugVerbosity::Scopes);

        if let Some(dsdt) = &acpi_context.dsdt {
            let dsdt_data = core::slice::from_raw_parts(
                phys_to_virt_addr(dsdt.address, dsdt.length as usize) as *const u8,
                dsdt.length as usize,
            );

            aml_context
                .parse_table(dsdt_data)
                .expect("Failed to parse DSDT");
        }

        for ssdt in &acpi_context.ssdts {
            let ssdt_data = core::slice::from_raw_parts(
                phys_to_virt_addr(ssdt.address, ssdt.length as usize) as *const u8,
                ssdt.length as usize,
            );

            aml_context
                .parse_table(ssdt_data)
                .expect("Failed to parse SSDT");
        }

        Self {
            acpi_context,
            aml_context,
            _marker: PhantomData,
        }
    }
}

pub static ACPI: Mutex<Option<Acpi<HandlerImpl>>> = Mutex::new(None);

pub unsafe fn init_bsp() {
    *ACPI.lock() = Some(Acpi::new(HandlerImpl));
}
