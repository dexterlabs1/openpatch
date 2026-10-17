//! Bounded, in-memory reading of legacy and current Rack patch containers.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

#[derive(Debug)]
pub struct VcvArchive {
    pub patch_json: Vec<u8>,
    pub assets: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Copy)]
struct Limits {
    compressed: usize,
    decoded: usize,
    member: usize,
    files: usize,
}

const LIMITS: Limits = Limits {
    compressed: 16 * 1024 * 1024,
    decoded: 64 * 1024 * 1024,
    member: 16 * 1024 * 1024,
    files: 1024,
};
const BLOCK: usize = 512;
const ZSTD_MAGIC: &[u8] = &[0x28, 0xb5, 0x2f, 0xfd];

/// Read one complete patch container without extracting anything to disk.
pub fn read(bytes: &[u8]) -> Result<VcvArchive, String> {
    read_with_limits(bytes, LIMITS)
}

fn read_with_limits(bytes: &[u8], limits: Limits) -> Result<VcvArchive, String> {
    if bytes.len() > limits.compressed {
        return Err("VCV input exceeds the compressed/raw input limit".into());
    }
    if !bytes.starts_with(ZSTD_MAGIC) {
        return json_archive(bytes, limits);
    }

    // A slice already implements BufRead: adding a BufReader would read ahead
    // and conceal unused compressed input. Consume the complete first frame so
    // checksum and truncation failures cannot be deferred to finish().
    let mut decoder = zstd::stream::read::Decoder::with_buffer(bytes)
        .map_err(|error| format!("Invalid VCV Zstd stream: {error}"))?
        .single_frame();
    decoder
        .window_log_max(26)
        .map_err(|error| format!("Cannot bound VCV Zstd window: {error}"))?;
    let mut decoded = Vec::new();
    decoder
        .by_ref()
        .take(limits.decoded as u64 + 1)
        .read_to_end(&mut decoded)
        .map_err(|error| format!("Invalid or incomplete VCV Zstd frame: {error}"))?;
    if decoded.len() > limits.decoded {
        return Err("VCV decoded data exceeds the limit".into());
    }
    decoder
        .finish_frame()
        .map_err(|error| format!("Invalid or incomplete VCV Zstd frame: {error}"))?;
    if !decoder.into_inner().is_empty() {
        return Err("VCV Zstd stream has trailing input or additional frames".into());
    }
    if decoded
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        == Some(b'{')
    {
        json_archive(&decoded, limits)
    } else {
        tar_archive(&decoded, limits)
    }
}

fn validate_json(bytes: &[u8]) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid VCV patch JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or("VCV patch JSON must be an object")?;
    if !object.get("modules").is_some_and(|value| value.is_array()) {
        return Err("VCV patch JSON must contain a modules array".into());
    }
    Ok(())
}

fn json_archive(bytes: &[u8], limits: Limits) -> Result<VcvArchive, String> {
    if bytes.len() > limits.member {
        return Err("VCV patch JSON exceeds the member limit".into());
    }
    validate_json(bytes)?;
    Ok(VcvArchive {
        patch_json: bytes.to_vec(),
        assets: BTreeMap::new(),
    })
}

/// Require a unique spelling rather than normalizing potentially unsafe names.
/// One terminal slash is allowed only for directory entries.
fn validate_path(path: &str, directory: bool) -> Result<&str, String> {
    if path.contains(['\\', '\0']) || path.starts_with('/') {
        return Err("Unsafe VCV archive member path".into());
    }
    let path = if directory {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    };
    if path.is_empty()
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || (part.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                    && part.as_bytes().get(1) == Some(&b':'))
        })
    {
        return Err("Unsafe or ambiguous VCV archive member path".into());
    }
    Ok(path)
}

fn header_path(header: &tar::Header) -> Vec<u8> {
    fn field(bytes: &[u8]) -> &[u8] {
        &bytes[..bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len())]
    }
    // tar's path_bytes() rewrites backslashes, even on Unix. Inspect the actual
    // path fields first so those spellings cannot disappear during parsing.
    let bytes = header.as_bytes();
    let mut path = Vec::new();
    if header.as_ustar().is_some() {
        let prefix = field(&bytes[345..500]);
        if !prefix.is_empty() {
            path.extend_from_slice(prefix);
            path.push(b'/');
        }
    }
    path.extend_from_slice(field(&bytes[..100]));
    path
}

fn metadata_path(bytes: &[u8]) -> Result<String, String> {
    let path = std::str::from_utf8(bytes).map_err(|_| "VCV member path is not UTF-8")?;
    validate_path(path, true)?;
    Ok(path.to_owned())
}

/// Validate physical members before tar is allowed to allocate extension data
/// or apply metadata. Also inspect all bytes after the end marker: a tar
/// iterator normally stops at its first zero block, concealing later members.
fn preflight_tar(bytes: &[u8], limits: Limits) -> Result<(), String> {
    if !bytes.len().is_multiple_of(BLOCK) {
        return Err("VCV tar data has an incomplete block".into());
    }
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let mut end = 0usize;
    let mut files = 0usize;
    let mut long_path: Option<String> = None;
    let mut pax_path: Option<String> = None;
    let mut pending_pax = false;
    for entry in archive
        .entries()
        .map_err(|error| format!("Invalid VCV tar archive: {error}"))?
        .raw(true)
    {
        let mut entry = entry.map_err(|error| format!("Invalid VCV tar member: {error}"))?;
        if entry.raw_header_position() != end as u64 {
            return Err("Unsupported VCV tar member layout".into());
        }
        let size = entry.size();
        if size > limits.member as u64 {
            return Err("VCV archive member exceeds the limit".into());
        }
        // Conversion and addition are bounded by both the member limit and
        // the actual decoded buffer; never allocate from a claimed tar size.
        let size = usize::try_from(size).map_err(|_| "Invalid VCV member size")?;
        let start = end.checked_add(BLOCK).ok_or("Invalid VCV member offset")?;
        let data_end = start.checked_add(size).ok_or("Invalid VCV member size")?;
        end = data_end
            .checked_add((BLOCK - size % BLOCK) % BLOCK)
            .filter(|end| *end <= bytes.len())
            .ok_or("Truncated VCV tar member")?;
        if bytes[data_end..end].iter().any(|byte| *byte != 0) {
            return Err("VCV tar member has nonzero padding".into());
        }
        let kind = entry.header().entry_type();
        if kind.is_gnu_longname() {
            if long_path.is_some() {
                return Err("Duplicate VCV GNU long-name metadata".into());
            }
            if entry.header().as_gnu().is_none() && entry.header().as_ustar().is_none() {
                return Err("Unsupported VCV long-name header format".into());
            }
            let data = &bytes[start..data_end];
            long_path = Some(metadata_path(data.strip_suffix(&[0]).unwrap_or(data))?);
            continue;
        }
        if kind.is_pax_local_extensions() {
            if pending_pax {
                return Err("Duplicate VCV PAX metadata".into());
            }
            if entry.header().as_gnu().is_none() && entry.header().as_ustar().is_none() {
                return Err("Unsupported VCV PAX header format".into());
            }
            pending_pax = true;
            let extensions = entry
                .pax_extensions()
                .map_err(|error| format!("Invalid VCV PAX metadata: {error}"))?
                .ok_or("Missing VCV PAX metadata")?;
            for extension in extensions {
                let extension =
                    extension.map_err(|error| format!("Invalid VCV PAX metadata: {error}"))?;
                match extension.key_bytes() {
                    b"path" => {
                        if pax_path.is_some() {
                            return Err("Duplicate VCV PAX path metadata".into());
                        }
                        pax_path = Some(metadata_path(extension.value_bytes())?);
                    }
                    b"size" | b"linkpath" => {
                        return Err("Unsupported VCV PAX size or link override".into());
                    }
                    key if key.starts_with(b"GNU.sparse") || key == b"SCHILY.realsize" => {
                        return Err("Unsupported VCV sparse archive metadata".into());
                    }
                    _ => {}
                }
            }
            continue;
        }
        if !kind.is_file() && !kind.is_dir() {
            return Err(
                "Unsupported VCV archive member type (links/special/global metadata)".into(),
            );
        }
        if kind.is_dir() && size != 0 {
            return Err("VCV archive directory contains data".into());
        }
        if kind.is_file() {
            files += 1;
            if files > limits.files {
                return Err("VCV archive exceeds the file count limit".into());
            }
        }
        if let (Some(long), Some(pax)) = (&long_path, &pax_path) {
            if long != pax {
                return Err("Ambiguous VCV archive path metadata".into());
            }
        }
        let path = header_path(entry.header());
        if path.contains(&b'\\') {
            return Err("Unsafe VCV archive member path".into());
        }
        match std::str::from_utf8(&path) {
            Ok(path) => {
                validate_path(path, kind.is_dir())?;
            }
            // A GNU long name may truncate a multibyte character in the
            // unused short header. The effective metadata path is checked.
            Err(_) if long_path.is_some() || pax_path.is_some() => {}
            Err(_) => return Err("VCV member path is not UTF-8".into()),
        }
        long_path = None;
        pax_path = None;
        pending_pax = false;
    }
    if long_path.is_some() || pending_pax {
        return Err("VCV archive has orphaned path metadata".into());
    }
    let padding = &bytes[end..];
    if padding.len() < 2 * BLOCK || padding.iter().any(|byte| *byte != 0) {
        return Err("VCV tar has a missing end marker or trailing members/data".into());
    }
    Ok(())
}

fn tar_archive(bytes: &[u8], limits: Limits) -> Result<VcvArchive, String> {
    preflight_tar(bytes, limits)?;
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let mut seen = BTreeMap::<String, bool>::new();
    let mut patch_json = None;
    let mut assets = BTreeMap::new();
    for entry in archive
        .entries()
        .map_err(|error| format!("Invalid VCV tar archive: {error}"))?
    {
        let mut entry = entry.map_err(|error| format!("Invalid VCV tar member: {error}"))?;
        let directory = entry.header().entry_type().is_dir();
        let path_bytes = entry.path_bytes();
        let path = std::str::from_utf8(&path_bytes).map_err(|_| "VCV member path is not UTF-8")?;
        let path = validate_path(path, directory)?.to_owned();
        if seen.contains_key(&path) {
            return Err(format!("Duplicate or ambiguous VCV member path: {path}"));
        }
        for (offset, _) in path.match_indices('/') {
            if seen.get(&path[..offset]) == Some(&false) {
                return Err("VCV archive file also acts as a directory".into());
            }
        }
        if !directory {
            let prefix = format!("{path}/");
            if seen
                .range(prefix.clone()..)
                .next()
                .is_some_and(|(existing, _)| existing.starts_with(&prefix))
            {
                return Err("VCV archive file also acts as a directory".into());
            }
        }
        seen.insert(path.clone(), directory);
        if directory {
            continue;
        }
        let mut data = Vec::new();
        entry
            .by_ref()
            .take(limits.member as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|error| format!("Cannot read VCV archive member: {error}"))?;
        if data.len() > limits.member {
            return Err("VCV archive member exceeds the limit".into());
        }
        if path == "patch.json" {
            patch_json = Some(data);
        } else {
            assets.insert(path, data);
        }
    }
    let patch_json = patch_json.ok_or("VCV archive must contain one root patch.json")?;
    validate_json(&patch_json)?;
    Ok(VcvArchive { patch_json, assets })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const PATCH: &[u8] = b" {\n  \"modules\": [], \"vendor\": {\"unknown\": 1.2300}\n}\n";

    fn member(path: &[u8], kind: u8, data: &[u8]) -> Vec<u8> {
        assert!(path.len() <= 100);
        let mut header = tar::Header::new_gnu();
        header.as_mut_bytes()[..100].fill(0);
        header.as_mut_bytes()[..path.len()].copy_from_slice(path);
        header.set_entry_type(tar::EntryType::new(kind));
        header.set_mode(0o644);
        header.set_size(data.len() as u64);
        header.set_cksum();
        let mut bytes = header.as_bytes().to_vec();
        bytes.extend_from_slice(data);
        bytes.resize(bytes.len().next_multiple_of(BLOCK), 0);
        bytes
    }

    fn archive(members: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = members.concat();
        bytes.resize(bytes.len() + 2 * BLOCK, 0);
        bytes
    }

    fn compress(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        encoder.include_checksum(true).unwrap();
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn assert_rejected(bytes: &[u8], message: &str) {
        let error = read(bytes).unwrap_err();
        assert!(
            error.contains(message),
            "{error:?} did not contain {message:?}"
        );
    }

    #[test]
    fn raw_and_direct_zstd_json_preserve_original_bytes() {
        for bytes in [PATCH.to_vec(), compress(PATCH)] {
            let result = read(&bytes).unwrap();
            assert_eq!(result.patch_json, PATCH);
            assert!(result.assets.is_empty());
        }
    }

    #[test]
    fn rack_format_tar_and_assets_preserve_original_bytes_and_paths() {
        let asset = b"\0\xffsample\n";
        let mut tar = archive(&[
            member(b"patch.json", b'0', PATCH),
            member(b"modules/", b'5', &[]),
            member("modules/42/音.wav".as_bytes(), b'0', asset),
        ]);
        // Real Rack saves can have record padding far beyond two end blocks.
        tar.resize(102_400, 0);
        let result = read(&compress(&tar)).unwrap();
        assert_eq!(result.patch_json, PATCH);
        assert_eq!(result.assets.len(), 1);
        assert_eq!(result.assets["modules/42/音.wav"], asset);
    }

    #[test]
    fn validates_json_envelope_without_rewriting_unknown_data() {
        for bytes in [
            &b"[]"[..],
            &b"{}"[..],
            &b"{\"modules\": {}}"[..],
            &b"{\"modules\": []"[..],
            &b"{\"modules\": []} {}"[..],
        ] {
            assert!(read(bytes).is_err());
            assert!(read(&compress(&archive(&[member(b"patch.json", b'0', bytes)]))).is_err());
        }
    }

    #[test]
    fn requires_one_root_patch_and_unique_member_names() {
        assert_rejected(
            &compress(&archive(&[member(b"nested/patch.json", b'0', PATCH)])),
            "one root patch.json",
        );
        for members in [
            vec![member(b"patch.json", b'0', PATCH); 2],
            vec![
                member(b"patch.json", b'0', PATCH),
                member(b"asset", b'0', b"a"),
                member(b"asset", b'0', b"b"),
            ],
            vec![
                member(b"patch.json", b'0', PATCH),
                member(b"dir/", b'5', b""),
                member(b"dir", b'0', b"a"),
            ],
        ] {
            assert_rejected(&compress(&archive(&members)), "Duplicate or ambiguous");
        }
        for paths in [[b"a".as_slice(), b"a/b".as_slice()], [b"a/b", b"a"]] {
            assert_rejected(
                &compress(&archive(&[
                    member(b"patch.json", b'0', PATCH),
                    member(paths[0], b'0', b"a"),
                    member(paths[1], b'0', b"b"),
                ])),
                "file also acts as a directory",
            );
        }
    }

    #[test]
    fn rejects_unsafe_paths_links_and_special_members() {
        for path in [
            &b"../escape"[..],
            &b"a/../escape"[..],
            &b"/absolute"[..],
            &b"C:/drive"[..],
            &b"C:drive"[..],
            &b"a\\b"[..],
            &b"./patch.json"[..],
            &b"a//b"[..],
            &b"a/./b"[..],
            &b"asset/"[..],
            &b"bad\xff"[..],
        ] {
            assert!(read(&compress(&archive(&[
                member(b"patch.json", b'0', PATCH),
                member(path, b'0', b"a"),
            ])))
            .is_err());
        }
        for kind in *b"12346SKg" {
            assert_rejected(
                &compress(&archive(&[
                    member(b"patch.json", b'0', PATCH),
                    member(b"special", kind, b""),
                ])),
                "Unsupported",
            );
        }
    }

    fn pax_record(key: &str, value: &str) -> Vec<u8> {
        let mut length = key.len() + value.len() + 4;
        loop {
            let record = format!("{length} {key}={value}\n");
            if record.len() == length {
                return record.into_bytes();
            }
            length = record.len();
        }
    }

    #[test]
    fn supports_long_gnu_and_pax_paths_and_validates_original_metadata() {
        let path = format!("assets/{}/音.wav", "long".repeat(30));
        let mut long = path.as_bytes().to_vec();
        long.push(0);
        for metadata in [
            member(b"././@LongLink", b'L', &long),
            member(b"PaxHeader", b'x', &pax_record("path", &path)),
        ] {
            let result = read(&compress(&archive(&[
                member(b"patch.json", b'0', PATCH),
                metadata,
                member(b"short-name", b'0', b"sample"),
            ])))
            .unwrap();
            assert_eq!(result.assets[&path], b"sample");
        }
        for path in ["../escape", "a\\b", "./patch.json", "C:/drive"] {
            for metadata in [
                member(b"././@LongLink", b'L', format!("{path}\0").as_bytes()),
                member(b"PaxHeader", b'x', &pax_record("path", path)),
            ] {
                assert!(read(&compress(&archive(&[
                    member(b"patch.json", b'0', PATCH),
                    metadata,
                    member(b"short-name", b'0', b"sample"),
                ])))
                .is_err());
            }
        }
        let duplicate = [pax_record("path", "a"), pax_record("path", "b")].concat();
        assert_rejected(
            &compress(&archive(&[
                member(b"PaxHeader", b'x', &duplicate),
                member(b"patch.json", b'0', PATCH),
            ])),
            "Duplicate VCV PAX path",
        );
        assert_rejected(
            &compress(&archive(&[
                member(b"PaxHeader", b'x', &pax_record("size", "999999999")),
                member(b"patch.json", b'0', PATCH),
            ])),
            "Unsupported VCV PAX size",
        );
    }

    #[test]
    fn rejects_incomplete_corrupt_and_additional_zstd_frames() {
        let compressed = compress(&archive(&[member(b"patch.json", b'0', PATCH)]));
        for removed in [1, 4, compressed.len() / 2] {
            assert_rejected(&compressed[..compressed.len() - removed], "Zstd");
        }
        let mut corrupt = compressed.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert_rejected(&corrupt, "Zstd");
        for suffix in [vec![0], compress(PATCH)] {
            let mut trailing = compressed.clone();
            trailing.extend(suffix);
            assert_rejected(&trailing, "trailing input");
        }
    }

    #[test]
    fn rejects_zstd_windows_larger_than_the_decoded_budget() {
        // A standard frame with a 2^27-byte window and a single raw block.
        // Its actual payload is tiny; rejection must come from the advertised
        // decoder window rather than an oversized expanded fixture.
        let mut bytes = ZSTD_MAGIC.to_vec();
        bytes.extend_from_slice(&[0, (27 - 10) << 3]);
        let block = ((PATCH.len() as u32) << 3) | 1;
        bytes.extend_from_slice(&block.to_le_bytes()[..3]);
        bytes.extend_from_slice(PATCH);
        assert_rejected(&bytes, "Zstd");
    }

    #[test]
    fn rejects_hidden_tar_members_and_incomplete_tar_framing() {
        let valid = archive(&[member(b"patch.json", b'0', PATCH)]);
        let mut hidden = valid.clone();
        hidden.extend(archive(&[member(b"hidden", b'0', b"asset")]));
        assert_rejected(&compress(&hidden), "trailing members");
        assert_rejected(
            &compress(&valid[..valid.len() - BLOCK]),
            "missing end marker",
        );
        assert_rejected(&compress(&valid[..valid.len() - 1]), "incomplete block");
        let mut corrupt_header = valid;
        corrupt_header[0] ^= 1;
        assert_rejected(&compress(&corrupt_header), "tar member");
    }

    #[test]
    fn enforces_each_bound_with_small_limits_and_exact_boundaries() {
        let tar = archive(&[
            member(b"patch.json", b'0', PATCH),
            member(b"asset", b'0', b"a"),
        ]);
        let compressed = compress(&tar);
        let exact = Limits {
            compressed: compressed.len(),
            decoded: tar.len(),
            member: PATCH.len(),
            files: 2,
        };
        assert!(read_with_limits(&compressed, exact).is_ok());
        for limits in [
            Limits {
                compressed: compressed.len() - 1,
                ..exact
            },
            Limits {
                decoded: tar.len() - 1,
                ..exact
            },
            Limits {
                member: PATCH.len() - 1,
                ..exact
            },
            Limits { files: 1, ..exact },
        ] {
            assert!(read_with_limits(&compressed, limits).is_err());
        }
        let exact_raw = Limits {
            compressed: PATCH.len(),
            member: PATCH.len(),
            ..LIMITS
        };
        assert!(read_with_limits(PATCH, exact_raw).is_ok());
        assert!(read_with_limits(
            PATCH,
            Limits {
                member: PATCH.len() - 1,
                ..exact_raw
            }
        )
        .is_err());
        let mut header = tar::Header::new_gnu();
        header.set_path("patch.json").unwrap();
        header.set_size(u64::MAX);
        header.set_cksum();
        assert!(read(&compress(&archive(&[header.as_bytes().to_vec()]))).is_err());
    }
}
