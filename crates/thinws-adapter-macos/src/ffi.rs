use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, OwnedFd};

#[repr(C)]
struct VolumeUuidBuffer {
    length: u32,
    returned: libc::attribute_set_t,
    uuid: libc::uuid_t,
}

pub(crate) fn file_system_type(fd: &OwnedFd) -> io::Result<String> {
    let mut stat = MaybeUninit::<libc::statfs>::uninit();

    // SAFETY: `fd` remains live and `stat` points to writable storage of the
    // exact type required by `fstatfs`; the output is read only after success.
    let result = unsafe { libc::fstatfs(fd.as_raw_fd(), stat.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: successful `fstatfs` initialized the fixed-size structure.
    let stat = unsafe { stat.assume_init() };
    Ok(decode_file_system_type(&stat.f_fstypename))
}

fn decode_file_system_type(value: &[libc::c_char]) -> String {
    let end = value
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(value.len());
    let bytes = value[..end]
        .iter()
        .map(|byte| *byte as u8)
        .collect::<Vec<_>>();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(crate) fn volume_uuid(fd: &OwnedFd) -> io::Result<Option<[u8; 16]>> {
    let mut attributes = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS,
        volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_UUID,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut buffer = MaybeUninit::<VolumeUuidBuffer>::zeroed();

    // SAFETY: `fd` is live; both pointers address writable C representations
    // of the declared sizes for the duration of this non-retaining syscall.
    let result = unsafe {
        libc::fgetattrlist(
            fd.as_raw_fd(),
            (&raw mut attributes).cast(),
            buffer.as_mut_ptr().cast(),
            size_of::<VolumeUuidBuffer>(),
            0,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: the buffer was zeroed and the successful syscall initialized all
    // returned fields. Length and returned-attribute bits gate UUID use below.
    let buffer = unsafe { buffer.assume_init() };
    Ok(decode_volume_uuid_buffer(&buffer))
}

fn decode_volume_uuid_buffer(buffer: &VolumeUuidBuffer) -> Option<[u8; 16]> {
    let uuid_end =
        size_of::<u32>() + size_of::<libc::attribute_set_t>() + size_of::<libc::uuid_t>();
    if usize::try_from(buffer.length).unwrap_or(0) < uuid_end
        || buffer.returned.volattr & libc::ATTR_VOL_UUID == 0
    {
        return None;
    }
    Some(buffer.uuid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_uuid_requires_both_returned_bit_and_complete_buffer() {
        let complete = u32::try_from(size_of::<VolumeUuidBuffer>()).unwrap();
        let value = [1_u8; 16];
        let mut buffer = VolumeUuidBuffer {
            length: complete,
            returned: libc::attribute_set_t {
                commonattr: 0,
                volattr: libc::ATTR_VOL_UUID,
                dirattr: 0,
                fileattr: 0,
                forkattr: 0,
            },
            uuid: value,
        };
        assert_eq!(decode_volume_uuid_buffer(&buffer), Some(value));

        buffer.returned.volattr = 0;
        assert_eq!(decode_volume_uuid_buffer(&buffer), None);
        buffer.returned.volattr = libc::ATTR_VOL_UUID;
        buffer.length = complete - 1;
        assert_eq!(decode_volume_uuid_buffer(&buffer), None);
    }

    #[test]
    fn filesystem_type_decode_is_bounded_when_the_fixed_buffer_has_no_nul() {
        let full = [b'a' as libc::c_char; 16];
        assert_eq!(decode_file_system_type(&full), "aaaaaaaaaaaaaaaa");

        let terminated = [
            b'a' as libc::c_char,
            b'p' as libc::c_char,
            b'f' as libc::c_char,
            b's' as libc::c_char,
            0,
            b'x' as libc::c_char,
        ];
        assert_eq!(decode_file_system_type(&terminated), "apfs");
    }
}
