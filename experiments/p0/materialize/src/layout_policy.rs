use thinws_p0_probe::{
    MaterializationPathReport, MaterializerCandidate, SupportState, VolumeRelation,
};

pub(crate) fn clone_layout_preconditions_satisfied(report: &MaterializationPathReport) -> bool {
    let paths = [
        &report.source,
        &report.target_root,
        &report.staging,
        &report.trash,
    ];
    paths.iter().all(|path| path.filesystem.type_name == "apfs")
        && report
            .pairs
            .iter()
            .all(|pair| pair.relation == VolumeRelation::SameVolume)
        && report.candidates.iter().any(|evidence| {
            evidence.candidate == MaterializerCandidate::FullCopy
                && evidence.state == SupportState::Supported
        })
}
