use super::*;

#[test]
fn cmd_preview_does_not_activate_during_row_editing() {
    let path = PathBuf::from("/tmp/preview.wav");

    assert_eq!(
        FileTable::preview_path_for_state(true, Some(&path), false),
        Some(path.clone())
    );
    assert_eq!(
        FileTable::preview_path_for_state(true, Some(&path), true),
        None
    );
}

#[test]
fn sub_threshold_preview_movement_seeks_once_on_release() {
    let path = PathBuf::from("/tmp/preview.wav");
    let mut scrub = Some(PreviewScrub::new(path.clone(), 0.2, 20., 100., None, None));

    assert!(scrub.as_mut().unwrap().update(&path, 0.23, 23.));
    assert_eq!(
        PreviewScrub::take_release_for_path(&mut scrub, Path::new("/tmp/other.wav")),
        None
    );
    assert!(scrub.is_some());
    assert_eq!(
        PreviewScrub::take_release_for_path(&mut scrub, &path),
        Some(PreviewPointerRelease::Seek(0.23))
    );
    assert!(scrub.is_none());
    assert_eq!(PreviewScrub::take_release_for_path(&mut scrub, &path), None);
}

#[test]
fn threshold_preview_movement_creates_normalized_trim() {
    let path = PathBuf::from("/tmp/preview.wav");
    let mut scrub = Some(PreviewScrub::new(path.clone(), 0.8, 80., 100., None, None));

    assert!(scrub.as_mut().unwrap().update(&path, 0.3, 30.));
    assert_eq!(
        PreviewScrub::take_release_for_path(&mut scrub, &path),
        Some(PreviewPointerRelease::Commit(
            TrimRange::new(0.3, 0.8).unwrap()
        ))
    );
}

#[test]
fn trim_handles_clamp_to_one_pixel_without_crossing() {
    let path = PathBuf::from("/tmp/preview.wav");
    let original = TrimRange::new(0.2, 0.8).unwrap();
    let mut start = PreviewScrub::new(
        path.clone(),
        0.2,
        20.,
        100.,
        Some(TrimEdge::Start),
        Some(original),
    );
    start.update(&path, 0.95, 95.);
    assert_eq!(
        start.provisional_trim(),
        Some(TrimRange::new(0.79, 0.8).unwrap())
    );

    let mut end = PreviewScrub::new(
        path.clone(),
        0.8,
        80.,
        100.,
        Some(TrimEdge::End),
        Some(original),
    );
    end.update(&path, 0.05, 5.);
    let adjusted = end.provisional_trim().unwrap();
    assert!((adjusted.start_ratio - 0.2).abs() < f32::EPSILON);
    assert!((adjusted.end_ratio - 0.21).abs() < 0.000_001);
}

#[test]
fn preview_playhead_atomic_round_trips_optional_ratio() {
    let bits = AtomicU32::new(u32::MAX);
    assert_eq!(FileTable::load_preview_playhead(&bits), None);

    FileTable::store_preview_playhead(&bits, Some(1.5));
    assert_eq!(FileTable::load_preview_playhead(&bits), Some(1.));

    FileTable::store_preview_playhead(&bits, None);
    assert_eq!(FileTable::load_preview_playhead(&bits), None);
}

#[test]
fn internal_drag_payload_updates_for_mouse_down_selection() {
    let drag = InternalFileDrag::new_shared(
        "first".to_string(),
        Arc::new(vec![PathBuf::from("/tmp/first.wav")]),
    );
    let drag_value = drag.clone();

    drag.replace(
        "selected".to_string(),
        vec![
            PathBuf::from("/tmp/first.wav"),
            PathBuf::from("/tmp/second.wav"),
        ],
    );

    let data = drag_value.snapshot();
    assert_eq!(data.label, "selected");
    assert_eq!(data.paths.len(), 2);
}

#[test]
fn native_drag_session_rejects_overlap_and_reopens_after_finish() {
    let session = NativeDragSession::default();

    assert!(session.try_start());
    assert!(session.is_active());
    assert!(!session.try_start());

    session.finish();

    assert!(session.try_start());
}
