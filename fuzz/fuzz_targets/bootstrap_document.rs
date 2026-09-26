#![no_main]

use std::str::FromStr;

use libfuzzer_sys::fuzz_target;
use thinws_adapter_macos::{
    decode_bootstrap_config, decode_root_marker, fuzz_workspace_ownership_document,
};
use thinws_core::{AbsolutePath, InstanceId, VolumeId};

fuzz_target!(|data: &[u8]| {
    if let Ok(identity) = decode_bootstrap_config(data) {
        assert_eq!(
            InstanceId::from_str(&identity.instance_id().to_string()).unwrap(),
            identity.instance_id()
        );
        assert_eq!(
            VolumeId::from_str(&identity.volume_id().to_string()).unwrap(),
            identity.volume_id()
        );
        assert_eq!(
            AbsolutePath::try_from_bytes(identity.data_root().as_bytes().to_vec()).unwrap(),
            *identity.data_root()
        );
    }
    if let Ok(marker) = decode_root_marker(data) {
        assert_eq!(
            AbsolutePath::try_from_bytes(marker.identity().data_root().as_bytes().to_vec())
                .unwrap(),
            *marker.identity().data_root()
        );
    }
    fuzz_workspace_ownership_document(data);
});
