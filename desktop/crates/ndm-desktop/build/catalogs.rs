use std::{collections::BTreeMap, env, fmt::Write, fs, path::PathBuf};

/// Build sorted static tables once. Drawing and switching languages require no I/O or parsing.
pub fn build() {
    let languages = [
        ("zh-Hant", "TraditionalChinese"),
        ("ja", "Japanese"),
        ("ko", "Korean"),
        ("es", "Spanish"),
        ("pt-BR", "BrazilianPortuguese"),
        ("fr", "French"),
        ("de", "German"),
    ];
    let mut catalogs = Vec::new();
    for (code, _) in languages {
        let path = format!("locales/{code}.json");
        println!("cargo:rerun-if-changed={path}");
        let data = fs::read(&path).expect("read UI translations");
        let catalog: BTreeMap<String, String> =
            serde_json::from_slice(&data).expect("parse UI translations");
        for (key, value) in &catalog {
            assert!(
                !value.trim().is_empty() && placeholders(key) == placeholders(value),
                "{path}: invalid translation for {key}"
            );
        }
        catalogs.push(catalog);
    }
    let keys: Vec<_> = catalogs[0].keys().collect();
    let mut output = format!("const KEYS: &[&str] = &{keys:?};\n");
    output.push_str("fn lookup(lang: Language, english: &str) -> Option<&'static str> {\nlet index = KEYS.binary_search(&english).ok()?;\nlet values: &[&str] = match lang {\n");
    for ((code, variant), catalog) in languages.into_iter().zip(&catalogs) {
        assert!(
            catalog.keys().eq(keys.iter().copied()),
            "different UI translation keys in {code}"
        );
        writeln!(
            output,
            "Language::{variant} => &{:?},",
            catalog.values().collect::<Vec<_>>()
        )
        .unwrap();
    }
    output.push_str("_ => return None,\n};\nSome(values[index])\n}\n");
    let path = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build directory"))
        .join("ui-catalogs.rs");
    fs::write(path, output).expect("write UI translations");
}

fn placeholders(value: &str) -> Vec<&str> {
    let mut keys: Vec<_> = value
        .split('{')
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(key, _)| key))
        .collect();
    keys.sort_unstable();
    keys
}
