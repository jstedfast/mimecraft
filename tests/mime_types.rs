// Adapted from MimeKit's UnitTests/MimeTypeTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{MimeTypeRegistrationError, MimeTypes};

#[test]
fn refreshed_filename_mappings_and_preferred_extensions() {
    let types = MimeTypes::new();
    for (extension, mime_type) in [
        (".3mf", "model/3mf"),
        (".aiff", "audio/x-aiff"),
        (".apng", "image/apng"),
        (".arrow", "application/vnd.apache.arrow.file"),
        (".avi", "video/x-msvideo"),
        (".cbor", "application/cbor"),
        (".class", "application/java-vm"),
        (".dart", "application/vnd.dart"),
        (".deb", "application/vnd.debian.binary-package"),
        (".dwg", "image/vnd.dwg"),
        (".dxf", "image/vnd.dxf"),
        (".eot", "application/vnd.ms-fontobject"),
        (".geojson", "application/geo+json"),
        (".glb", "model/gltf-binary"),
        (".gltf", "model/gltf+json"),
        (".hqx", "application/mac-binhex40"),
        (".jp2", "image/jp2"),
        (".jpf", "image/jpx"),
        (".m3u", "audio/x-mpegurl"),
        (".m3u8", "application/vnd.apple.mpegurl"),
        (".mif", "application/vnd.mif"),
        (".mka", "audio/matroska"),
        (".mkv", "video/matroska"),
        (".obj", "model/obj"),
        (".odf", "application/vnd.oasis.opendocument.formula"),
        (".odg", "application/vnd.oasis.opendocument.graphics"),
        (
            ".otg",
            "application/vnd.oasis.opendocument.graphics-template",
        ),
        (
            ".otp",
            "application/vnd.oasis.opendocument.presentation-template",
        ),
        (
            ".ots",
            "application/vnd.oasis.opendocument.spreadsheet-template",
        ),
        (".ott", "application/vnd.oasis.opendocument.text-template"),
        (".p12", "application/pkcs12"),
        (".parquet", "application/vnd.apache.parquet"),
        (".pcl", "application/vnd.hp-PCL"),
        (".pnm", "image/x-portable-anymap"),
        (".psd", "image/vnd.adobe.photoshop"),
        (".py", "text/x-script.python"),
        (".roff", "text/troff"),
        (".smil", "application/smil+xml"),
        (".sqlite3", "application/vnd.sqlite3"),
        (".step", "model/step"),
        (".stl", "model/stl"),
        (".vsd", "application/vnd.visio"),
        (".vtt", "text/vtt"),
        (".webmanifest", "application/manifest+json"),
        (".wpd", "application/vnd.wordperfect"),
        (".wrl", "model/vrml"),
        (".xls", "application/vnd.ms-excel"),
        (".xpm", "image/x-xpixmap"),
    ] {
        assert_eq!(types.mime_type(&format!("file{extension}")), mime_type);
        assert_eq!(
            types.mime_type(&format!("file{}", extension.to_ascii_uppercase())),
            mime_type
        );
        assert_eq!(types.extension(mime_type), Some(extension));
        assert_eq!(
            types.extension(&mime_type.to_ascii_uppercase()),
            Some(extension)
        );
    }
}

#[test]
fn refreshed_alternate_extensions_use_the_independent_preferred_mapping() {
    let types = MimeTypes::new();
    for (extension, mime_type, preferred) in [
        (".aif", "audio/x-aiff", ".aiff"),
        (".aifc", "audio/x-aiff", ".aiff"),
        (".asx", "video/x-ms-asf", ".asf"),
        (".au", "audio/basic", ".snd"),
        (".jpg2", "image/jp2", ".jp2"),
        (".jpx", "image/jpx", ".jpf"),
        (".man", "text/troff", ".roff"),
        (".me", "text/troff", ".roff"),
        (".ms", "text/troff", ".roff"),
        (".opus", "audio/ogg", ".ogg"),
        (".pfx", "application/pkcs12", ".p12"),
        (".pm", "image/x-xpixmap", ".xpm"),
        (".smi", "application/smil+xml", ".smil"),
        (".sml", "application/smil+xml", ".smil"),
        (".sqlite", "application/vnd.sqlite3", ".sqlite3"),
        (".stp", "model/step", ".step"),
        (".t", "text/troff", ".roff"),
        (".tr", "text/troff", ".roff"),
        (".udeb", "application/vnd.debian.binary-package", ".deb"),
        (".vrml", "model/vrml", ".wrl"),
        (".vss", "application/vnd.visio", ".vsd"),
        (".vst", "application/vnd.visio", ".vsd"),
        (".vsw", "application/vnd.visio", ".vsd"),
        (".w60", "application/vnd.wordperfect", ".wpd"),
        (".w61", "application/vnd.wordperfect", ".wpd"),
        (".wp", "application/vnd.wordperfect", ".wpd"),
        (".wp5", "application/vnd.wordperfect", ".wpd"),
        (".wp6", "application/vnd.wordperfect", ".wpd"),
        (".wrz", "model/vrml", ".wrl"),
        (".wsc", "text/scriptlet", ".sct"),
        (".xl", "application/vnd.ms-excel", ".xls"),
        (".xlb", "application/vnd.ms-excel", ".xls"),
        (".xlc", "application/vnd.ms-excel", ".xls"),
        (".xld", "application/vnd.ms-excel", ".xls"),
        (".xlk", "application/vnd.ms-excel", ".xls"),
        (".xll", "application/vnd.ms-excel", ".xls"),
        (".xlm", "application/vnd.ms-excel", ".xls"),
        (".xlv", "application/vnd.ms-excel", ".xls"),
        (".xlw", "application/vnd.ms-excel", ".xls"),
    ] {
        assert_eq!(types.mime_type(&format!("file{extension}")), mime_type);
        assert_eq!(
            types.mime_type(&format!("file{}", extension.to_ascii_uppercase())),
            mime_type
        );
        assert_eq!(types.extension(mime_type), Some(preferred));
        assert_eq!(
            types.extension(&mime_type.to_ascii_uppercase()),
            Some(preferred)
        );
    }
}

#[test]
fn legacy_reverse_aliases_and_ambiguous_filename_mappings_remain() {
    let types = MimeTypes::new();
    for (mime_type, extension) in [
        ("video/x-matroska", ".mkv"),
        ("application/sla", ".stl"),
        ("application/acad", ".dwg"),
        ("application/binhex", ".hqx"),
        ("application/dart", ".dart"),
        ("application/dxf", ".dxf"),
        ("application/excel", ".xls"),
        ("application/pkcs-12", ".p12"),
        ("application/smil", ".smi"),
        ("application/step", ".step"),
        ("application/vnd.bw-fontobject", ".eot"),
        ("application/wordperfect", ".wp5"),
        ("application/wordperfect6.0", ".w60"),
        ("application/wordperfect6.1", ".w61"),
        ("application/x-deb", ".deb"),
        ("application/x-debian-package", ".deb"),
        ("application/x-java-applet", ".class"),
        ("application/x-mif", ".mif"),
        ("application/x-mplayer2", ".asx"),
        ("application/x-pcl", ".pcl"),
        ("application/x-pkcs12", ".pfx"),
        ("application/x-portable-anymap", ".pnm"),
        ("application/x-sqlite3", ".sqlite3"),
        ("application/x-troff", ".roff"),
        ("application/x-troff-man", ".man"),
        ("application/x-troff-me", ".me"),
        ("application/x-troff-ms", ".ms"),
        ("application/x-visio", ".vsd"),
        ("application/x-vrml", ".vrml"),
        ("application/x-world", ".svr"),
        ("audio/aiff", ".aiff"),
        ("audio/opus", ".opus"),
        ("audio/x-au", ".au"),
        ("audio/x-matroska", ".mka"),
        ("audio/x-mpequrl", ".m3u"),
        ("image/xpm", ".xpm"),
        ("text/scriplet", ".wsc"),
        ("text/x-script.phyton", ".py"),
        ("video/avi", ".avi"),
    ] {
        assert_eq!(types.extension(mime_type), Some(extension));
        assert_eq!(
            types.extension(&mime_type.to_ascii_uppercase()),
            Some(extension)
        );
    }
    for (extension, mime_type) in [
        (".ts", "application/typescript"),
        (".rpm", "audio/x-pn-realaudio-plugin"),
        (".xml", "text/xml"),
        (".rtf", "text/rtf"),
        (".db", "application/octet-stream"),
    ] {
        assert_eq!(types.mime_type(&format!("file{extension}")), mime_type);
    }
}

#[test]
fn canonical_filename_and_preferred_extension_examples() {
    let types = MimeTypes::new();
    for (filename, expected) in [
        ("filename.txt", "text/plain"),
        ("filename.csv", "text/csv"),
        ("filename.eml", "message/rfc822"),
        ("photo.jpeg", "image/jpeg"),
        ("photo.JPG", "image/jpeg"),
        ("index.htm", "text/html"),
        ("index.HTML", "text/html"),
        ("archive.tar.gz", "application/gzip"),
        ("archive.tar", "application/x-tar"),
    ] {
        assert_eq!(types.mime_type(filename), expected, "{filename:?}");
    }
    for (mime_type, expected) in [
        ("text/plain", ".txt"),
        ("TEXT/CSV", ".csv"),
        ("message/rfc822", ".eml"),
        ("image/JPEG", ".jpg"),
        ("application/octet-stream", ".bin"),
    ] {
        assert_eq!(types.extension(mime_type), Some(expected), "{mime_type:?}");
    }
}

#[test]
fn iana_additions_support_filename_and_preferred_extension_lookup() {
    let types = MimeTypes::new();
    for (extension, mime_type) in [
        (".wasm", "application/wasm"),
        (".yaml", "application/yaml"),
        (".toml", "application/toml"),
        (".jxl", "image/jxl"),
        (".heic", "image/heic"),
        (".heics", "image/heic-sequence"),
        (".heif", "image/heif"),
        (".heifs", "image/heif-sequence"),
        (".zst", "application/zstd"),
        (".avif", "image/avif"),
        (".jsonld", "application/ld+json"),
    ] {
        assert_eq!(types.mime_type(&format!("file{extension}")), mime_type);
        assert_eq!(types.extension(mime_type), Some(extension));
        assert_eq!(
            types.mime_type(&format!("file{}", extension.to_ascii_uppercase())),
            mime_type
        );
        assert_eq!(
            types.extension(&mime_type.to_ascii_uppercase()),
            Some(extension)
        );
    }
    // Both YAML suffixes are registered, but IANA explicitly prefers .yaml.
    assert_eq!(types.mime_type("config.yml"), "application/yaml");
    assert_eq!(types.mime_type("config.YML"), "application/yaml");
    assert_eq!(types.extension("application/yaml"), Some(".yaml"));
}

#[test]
fn registered_gzip_and_flac_names_retain_legacy_reverse_aliases() {
    let mut types = MimeTypes::new();
    for (filename, expected) in [
        ("archive.gz", "application/gzip"),
        ("archive.GZIP", "application/gzip"),
        ("archive.tar.gz", "application/gzip"),
        ("music.flac", "audio/flac"),
        ("music.FLAC", "audio/flac"),
    ] {
        assert_eq!(types.mime_type(filename), expected);
    }
    for (mime_type, extension) in [
        ("application/gzip", ".gz"),
        ("application/x-gzip", ".gz"),
        ("audio/flac", ".flac"),
        ("audio/x-flac", ".flac"),
    ] {
        assert_eq!(types.extension(mime_type), Some(extension));
        assert_eq!(
            types.extension(&mime_type.to_ascii_uppercase()),
            Some(extension)
        );
    }

    types.register("application/x-gzip", ".gz").unwrap();
    assert_eq!(types.mime_type("archive.gz"), "application/x-gzip");
    assert_eq!(types.mime_type("archive.gzip"), "application/gzip");
    assert_eq!(types.extension("application/gzip"), Some(".gz"));
    types.register("audio/x-flac", ".flac").unwrap();
    assert_eq!(types.mime_type("music.flac"), "audio/x-flac");
    assert_eq!(types.extension("audio/flac"), Some(".flac"));
    assert_eq!(MimeTypes::new().mime_type("music.flac"), "audio/flac");
}

#[test]
fn absent_and_unknown_extensions_use_the_binary_fallback() {
    let types = MimeTypes::default();
    for filename in [
        "",
        "filename",
        "filename.",
        ".",
        "..",
        "filename.bogus",
        "parent.txt/filename",
        r"parent.txt\filename",
        "file.txt/",
        "file.txt\\",
        "file.txt/.",
        "file.txt/..",
        "file.txt/child.",
        "file.txt ",
        "file.txt?version=1",
        "file.txt#fragment",
        "file%2etxt",
        "file.txt\0",
        "file.\u{ff34}\u{ff38}\u{ff34}",
    ] {
        assert_eq!(
            types.mime_type(filename),
            "application/octet-stream",
            "{filename:?}"
        );
    }
}

#[test]
fn extension_extraction_is_portable_and_uses_only_the_final_dot() {
    let types = MimeTypes::new();
    for filename in [
        ".txt",
        "..txt",
        "file..txt",
        "/parent.csv/file.txt",
        r"C:\parent.csv\file.txt",
        r"\\server\parent.csv\file.txt",
        r"C:\mixed/path.csv\file.txt",
        r"parent.csv\child/.txt",
        "file.txt.exe.txt",
        "r\u{e9}sum\u{e9}.TXT",
    ] {
        assert_eq!(types.mime_type(filename), "text/plain", "{filename:?}");
    }
}

#[test]
fn preferred_extensions_require_a_literal_type_name() {
    let types = MimeTypes::new();
    for mime_type in [
        "",
        "application/x-vnd.fake-mime-type",
        "text/plain; charset=utf-8",
        " text/plain",
        "text/plain ",
        "plain",
        "text/*",
        "text/plain\0",
    ] {
        assert_eq!(types.extension(mime_type), None, "{mime_type:?}");
    }
}

#[test]
fn registration_adds_a_dot_and_preserves_values_but_folds_ascii_keys() {
    let mut types = MimeTypes::new();
    assert_eq!(
        types.mime_type("filename.bogus"),
        "application/octet-stream"
    );
    assert_eq!(types.extension("application/vnd.bogus"), None);

    types.register("Application/Vnd.Bogus", ".BoGuS").unwrap();
    assert_eq!(types.mime_type("filename.BOGUS"), "Application/Vnd.Bogus");
    assert_eq!(types.extension("APPLICATION/VND.BOGUS"), Some(".BoGuS"));

    types.register("application/vnd.no-dot", "NoDot").unwrap();
    assert_eq!(types.mime_type("filename.nodot"), "application/vnd.no-dot");
    assert_eq!(types.extension("application/vnd.no-dot"), Some(".NoDot"));

    types.register("APPLICATION/VND.BOGUS", ".bogus").unwrap();
    assert_eq!(types.mime_type("filename.BoGuS"), "APPLICATION/VND.BOGUS");
    assert_eq!(types.extension("Application/Vnd.Bogus"), Some(".bogus"));
}

#[test]
fn registration_overwrites_each_direction_without_removing_old_aliases() {
    let mut types = MimeTypes::new();
    types.register("image/jpeg", ".JfIfCustom").unwrap();
    assert_eq!(types.extension("IMAGE/JPEG"), Some(".JfIfCustom"));
    assert_eq!(types.mime_type("photo.jfifcustom"), "image/jpeg");
    assert_eq!(types.mime_type("photo.jpg"), "image/jpeg");
    assert_eq!(types.mime_type("photo.jpeg"), "image/jpeg");

    types.register("application/vnd.custom", "JPG").unwrap();
    assert_eq!(types.mime_type("photo.jpg"), "application/vnd.custom");
    assert_eq!(types.extension("application/vnd.custom"), Some(".JPG"));
    assert_eq!(types.extension("image/jpeg"), Some(".JfIfCustom"));
    assert_eq!(types.mime_type("photo.jpeg"), "image/jpeg");

    types
        .register("application/vnd.other", "JfIfCustom")
        .unwrap();
    assert_eq!(types.mime_type("photo.jfifcustom"), "application/vnd.other");
    // A previous reverse mapping remains even when its extension is reassigned.
    assert_eq!(types.extension("image/jpeg"), Some(".JfIfCustom"));
    assert_eq!(
        types.extension("application/vnd.other"),
        Some(".JfIfCustom")
    );
    assert_eq!(types.extension("application/vnd.custom"), Some(".JPG"));
}

#[test]
fn instances_and_clones_do_not_share_mutable_registrations() {
    let mut first = MimeTypes::new();
    let second = MimeTypes::default();
    first.register("application/vnd.first", "txt").unwrap();
    let mut cloned = first.clone();
    first.register("application/vnd.changed", "txt").unwrap();
    cloned.register("application/vnd.clone", "csv").unwrap();

    assert_eq!(first.mime_type("file.txt"), "application/vnd.changed");
    assert_eq!(cloned.mime_type("file.txt"), "application/vnd.first");
    assert_eq!(second.mime_type("file.txt"), "text/plain");
    assert_eq!(MimeTypes::new().mime_type("file.txt"), "text/plain");
    assert_eq!(first.mime_type("file.csv"), "text/csv");
    assert_eq!(cloned.mime_type("file.csv"), "application/vnd.clone");
    assert_eq!(second.extension("application/vnd.first"), None);
    assert_eq!(cloned.extension("application/vnd.changed"), None);
    assert_eq!(first.extension("application/vnd.clone"), None);
}

#[test]
fn empty_registration_inputs_report_errors_without_mutation() {
    let mut types = MimeTypes::new();
    types.register("text/plain", ".custom").unwrap();
    for (mime_type, extension, expected) in [
        ("", ".custom", MimeTypeRegistrationError::EmptyMimeType),
        ("text/plain", "", MimeTypeRegistrationError::EmptyExtension),
        ("", "", MimeTypeRegistrationError::EmptyMimeType),
    ] {
        assert_eq!(types.register(mime_type, extension), Err(expected));
        assert_eq!(types.mime_type("file.custom"), "text/plain");
        assert_eq!(types.extension("text/plain"), Some(".custom"));
        assert_eq!(types.mime_type("file.txt"), "text/plain");
        assert_eq!(types.extension(""), None);
    }
    assert_eq!(
        MimeTypeRegistrationError::EmptyMimeType.to_string(),
        "cannot register an empty MIME-type"
    );
    assert_eq!(
        MimeTypeRegistrationError::EmptyExtension.to_string(),
        "cannot register an empty file extension"
    );
}

#[test]
fn registration_is_not_syntax_validation_or_whitespace_normalization() {
    let mut types = MimeTypes::new();
    types
        .register(" not/a type; parameter=value ", " custom ")
        .unwrap();
    assert_eq!(
        types.mime_type("file. custom "),
        " not/a type; parameter=value "
    );
    assert_eq!(
        types.extension(" NOT/A TYPE; PARAMETER=VALUE "),
        Some(". custom ")
    );
    assert_eq!(types.extension("not/a type; parameter=value"), None);
    assert_eq!(types.mime_type("file.custom"), "application/octet-stream");

    types.register(" ", " ").unwrap();
    assert_eq!(types.mime_type("file. "), " ");
    assert_eq!(types.extension(" "), Some(". "));

    for extension in [".", "..txt", ".tar.gz", ".dir/txt", ".dir\\txt"] {
        types
            .register("application/vnd.literal", extension)
            .unwrap();
        assert_eq!(types.extension("application/vnd.literal"), Some(extension));
    }
    assert_eq!(types.mime_type("file."), "application/octet-stream");
    assert_eq!(types.mime_type("file..txt"), "text/plain");
    assert_eq!(types.mime_type("file.tar.gz"), "application/gzip");
    assert_eq!(types.mime_type("file.dir/txt"), "application/octet-stream");
    assert_eq!(types.mime_type(r"file.dir\txt"), "application/octet-stream");
}

#[test]
fn unicode_registration_keys_are_literal_except_for_ascii_case() {
    let mut types = MimeTypes::new();
    types
        .register("application/x-\u{e9}Xample", "\u{e9}XT")
        .unwrap();
    assert_eq!(
        types.mime_type("file.\u{e9}xt"),
        "application/x-\u{e9}Xample"
    );
    assert_eq!(
        types.extension("APPLICATION/X-\u{e9}XAMPLE"),
        Some(".\u{e9}XT")
    );
    assert_eq!(types.mime_type("file.\u{c9}XT"), "application/octet-stream");
    assert_eq!(types.extension("application/x-\u{c9}xample"), None);
    types
        .register("application/x-\u{c9}Xample", "\u{c9}XT")
        .unwrap();
    assert_eq!(
        types.mime_type("file.\u{c9}xt"),
        "application/x-\u{c9}Xample"
    );
    assert_eq!(
        types.mime_type("file.\u{e9}xt"),
        "application/x-\u{e9}Xample"
    );
}
