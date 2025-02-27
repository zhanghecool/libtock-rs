use crate::kernel_data::with_kernel_data;
use crate::{ExpectedSyscall, SyscallLogEntry};
use libtock_platform::{return_variant, ErrorCode, Register};
use std::convert::TryInto;

pub(super) unsafe fn allow_ro(
    driver_number: Register,
    buffer_number: Register,
    address: Register,
    len: Register,
) -> [Register; 4] {
    let driver_number = driver_number.try_into().expect("Too large driver number");
    let buffer_number = buffer_number.try_into().expect("Too large buffer number");
    let result = with_kernel_data(|option_kernel_data| {
        let kernel_data =
            option_kernel_data.expect("Read-Only Allow called but no fake::Kernel exists");

        kernel_data.syscall_log.push(SyscallLogEntry::AllowRo {
            driver_number,
            buffer_number,
            len: len.into(),
        });

        // Check for an expected syscall entry. Returns an error from the lambda
        // if this syscall was expected and return_error was specified. Panics
        // if a different syscall was expected.
        match kernel_data.expected_syscalls.pop_front() {
            None => {}
            Some(ExpectedSyscall::AllowRo {
                driver_number: expected_driver_number,
                buffer_number: expected_buffer_number,
                return_error,
            }) => {
                assert_eq!(
                    driver_number, expected_driver_number,
                    "expected different driver_number"
                );
                assert_eq!(
                    buffer_number, expected_buffer_number,
                    "expected different buffer_number"
                );
                if let Some(error_code) = return_error {
                    return Err(error_code);
                }
            }
            Some(expected_syscall) => expected_syscall.panic_wrong_call("Read-Only Allow"),
        };

        let driver = match kernel_data.drivers.get(&driver_number) {
            None => return Err(ErrorCode::NoDevice),
            Some(driver_data) => driver_data.driver.clone(),
        };

        // Safety: RawSyscall requires the caller to specify address and len as
        // required by TRD 104. That trivially satisfies the precondition of
        // insert_ro_buffer, which also requires address and len to follow TRD
        // 104.
        let buffer = unsafe { kernel_data.allow_db.insert_ro_buffer(address, len) }
            .expect("Read-Only Allow called with a buffer that overlaps an already-Allowed buffer");

        Ok((driver, buffer))
    });

    let (driver, buffer) = match result {
        Ok((driver, buffer)) => (driver, buffer),
        Err(error_code) => {
            let r0: u32 = return_variant::FAILURE_2_U32.into();
            let r1: u32 = error_code as u32;
            return [r0.into(), r1.into(), address, len];
        }
    };

    let (error_code, buffer_out) = match driver.allow_readonly(buffer_number, buffer) {
        Ok(buffer_out) => (None, buffer_out),
        Err((buffer_out, error_code)) => (Some(error_code), buffer_out),
    };

    let (address_out, len_out) = with_kernel_data(|option_kernel_data| {
        let kernel_data = option_kernel_data
            .expect("fake::Kernel dropped during fake::SyscallDriver::allow_readonly");
        kernel_data.allow_db.remove_ro_buffer(buffer_out)
    });

    match error_code {
        None => {
            let r0: u32 = return_variant::SUCCESS_2_U32.into();
            // The value of r3 isn't specified in TRD 104, but in practice the
            // kernel won't change it. This mimics that behavior, for lack of a
            // better option.
            [r0.into(), address_out, len_out, len]
        }
        Some(error_code) => {
            let r0: u32 = return_variant::FAILURE_2_U32.into();
            let r1: u32 = error_code as u32;
            [r0.into(), r1.into(), address_out, len_out]
        }
    }
}
