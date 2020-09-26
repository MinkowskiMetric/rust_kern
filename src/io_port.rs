use core::marker::PhantomData;

pub trait Io {
    type Value: Copy;

    fn write(&mut self, value: Self::Value);
    fn read(&self) -> Self::Value;
}

pub struct IoPort<T> {
    port: u16,
    _marker: PhantomData<T>,
}

impl<T> IoPort<T> {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            _marker: PhantomData,
        }
    }
}

impl Io for IoPort<u8> {
    type Value = u8;

    #[inline(always)]
    fn read(&self) -> Self::Value {
        let value: u8;
        unsafe {
            asm!(
                "in al, dx",
                out("al") value,
                in("dx") self.port,
                options(nomem)
            );
        }
        value
    }

    #[inline(always)]
    fn write(&mut self, value: Self::Value) {
        unsafe {
            asm!(
                "out dx, al",
                in("al") value,
                in("dx") self.port,
                options(nomem)
            );
        }
    }
}

impl Io for IoPort<u16> {
    type Value = u16;

    #[inline(always)]
    fn read(&self) -> Self::Value {
        let value: u16;
        unsafe {
            asm!(
                "in ax, dx",
                out("ax") value,
                in("dx") self.port,
                options(nomem)
            );
        }
        value
    }

    #[inline(always)]
    fn write(&mut self, value: Self::Value) {
        unsafe {
            asm!(
                "out ax, dx",
                in("ax") value,
                in("dx") self.port,
                options(nomem)
            );
        }
    }
}

impl Io for IoPort<u32> {
    type Value = u32;

    #[inline(always)]
    fn read(&self) -> Self::Value {
        let value: u32;
        unsafe {
            asm!(
                "in eax, dx",
                out("eax") value,
                in("dx") self.port,
                options(nomem)
            );
        }
        value
    }

    #[inline(always)]
    fn write(&mut self, value: Self::Value) {
        unsafe {
            asm!(
                "out eax, dx",
                in("eax") value,
                in("dx") self.port,
                options(nomem)
            );
        }
    }
}
