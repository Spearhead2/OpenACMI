//! Frame decimation for very large recordings.
//!
//! Keeps one in every `keep_every_nth` frames, where a frame is a `#time`
//! line and everything up to the next one. Loading is lossless by default;
//! decimation is an opt-in trade of temporal resolution for memory.
//!
//! Frames that carry structural information are *protected* and always kept:
//!
//! * the header block before the first frame,
//! * the first appearance of any object id,
//! * object removals (`-id`),
//! * events (`Event=`).

use std::ops::Range;

use rustc_hash::FxHashSet;

/// Returns the byte ranges of `text` to feed to the parser, in order.
///
/// Adjacent kept frames are merged, so with `keep_every_nth <= 1` the result
/// is a single range covering the whole text.
pub fn decimate_frames(text: &str, keep_every_nth: usize) -> Vec<Range<usize>> {
    if keep_every_nth <= 1 {
        return std::iter::once(0..text.len()).collect();
    }

    let mut kept: Vec<Range<usize>> = Vec::new();
    let mut seen_ids: FxHashSet<&str> = FxHashSet::default();
    let mut unprotected = 0usize;

    let mut frame_start = 0;
    let mut protected = true; // the header block
    let mut offset = 0;

    let mut close_frame = |range: Range<usize>, protected: bool, kept: &mut Vec<Range<usize>>| {
        if range.is_empty() {
            return;
        }
        let keep = protected || {
            unprotected += 1;
            unprotected % keep_every_nth == 1
        };
        if keep {
            match kept.last_mut() {
                Some(last) if last.end == range.start => last.end = range.end,
                _ => kept.push(range),
            }
        }
    };

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start_matches('\u{feff}').trim();
        if trimmed.starts_with('#') {
            close_frame(frame_start..offset, protected, &mut kept);
            frame_start = offset;
            protected = false;
        } else if !protected {
            protected = is_protected_line(trimmed, &mut seen_ids);
        } else {
            // Still record ids so later frames do not count as first appearances.
            is_protected_line(trimmed, &mut seen_ids);
        }
        offset += line.len();
    }
    close_frame(frame_start..offset, protected, &mut kept);
    kept
}

fn is_protected_line<'a>(line: &'a str, seen_ids: &mut FxHashSet<&'a str>) -> bool {
    if line.is_empty() || line.starts_with("//") {
        return false;
    }
    if line.starts_with('-') {
        return true;
    }
    let Some((id, rest)) = line.split_once(',') else { return false };
    let id = id.trim();
    let first_appearance = id != "0" && seen_ids.insert(id);
    first_appearance || rest.contains("Event=")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept_text(text: &str, n: usize) -> String {
        decimate_frames(text, n).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn lossless_when_disabled() {
        let text = "a\n#1\nb\n";
        assert_eq!(decimate_frames(text, 1), vec![0..text.len()]);
        assert_eq!(decimate_frames(text, 0), vec![0..text.len()]);
    }

    #[test]
    fn keeps_protected_frames_and_every_nth() {
        let text = "FileType=text/acmi/tacview\nFileVersion=2.2\n\
                    #0\n1,T=0|0|0\n\
                    #1\n1,T=1|0|0\n\
                    #2\n1,T=2|0|0\n\
                    #3\n1,T=3|0|0\n\
                    #4\n1,T=4|0|0\n0,Event=Bookmark|x\n\
                    #5\n1,T=5|0|0\n\
                    #6\n-1\n";
        let out = kept_text(text, 3);
        // Header, first appearance (#0), unprotected #1 (1st of its group of 3),
        // event frame (#4), unprotected #5 (4th → starts the next group), removal (#6).
        assert!(out.starts_with("FileType="));
        for frame in ["#0\n", "#1\n", "#4\n", "#5\n", "#6\n-1\n"] {
            assert!(out.contains(frame), "missing {frame:?} in {out:?}");
        }
        for frame in ["#2\n", "#3\n"] {
            assert!(!out.contains(frame), "unexpected {frame:?} in {out:?}");
        }
    }
}
