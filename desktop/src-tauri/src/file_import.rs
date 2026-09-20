use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs::File, io::Read, path::Path};

pub const FIXED_PATTERN: &str = "dutch ? ? ? fog ? ? ? ? ? ? parrot";
pub const MAX_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_ROWS: usize = 100_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRow {
    pub line: usize,
    pub words: Vec<String>,
    pub pool: String,
    pub error: Option<String>,
    pub duplicate_of: Option<usize>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub import_id: String,
    pub filename: String,
    pub rows: Vec<ImportRow>,
    pub valid_count: usize,
    pub invalid_count: usize,
    pub duplicate_count: usize,
}

pub fn parse_text(text: &str, filename: &str) -> Result<ImportPreview, String> {
    if text.len() > MAX_BYTES {
        return Err("O texto extraído excede 10 MB.".into());
    }
    let mut seen = HashMap::new();
    let mut rows = Vec::new();
    let text = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace(['\r', '\u{b}', '\u{c}'], "\n");
    let dictionary = bip39::Language::English.word_list();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if rows.len() >= MAX_ROWS {
            return Err("O arquivo deve ter no máximo 100.000 linhas preenchidas.".into());
        }
        let words: Vec<String> = line
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        let mut row = ImportRow {
            line: index + 1,
            words,
            pool: String::new(),
            error: None,
            duplicate_of: None,
        };
        if row.words.len() != 12 {
            row.error = Some(format!(
                "Encontradas {} palavras; são necessárias exatamente 12.",
                row.words.len()
            ));
        } else if let Some(word) = row
            .words
            .iter()
            .find(|word| dictionary.binary_search(&word.as_str()).is_err())
        {
            row.error = Some(format!(
                "'{word}' não pertence à lista BIP-39 em inglês. Use palavras sem @ ou números."
            ));
        } else {
            let mut pool = row.words.clone();
            for anchor in ["dutch", "fog", "parrot"] {
                if let Some(position) = pool.iter().position(|word| word == anchor) {
                    pool.remove(position);
                } else {
                    row.error = Some(format!("Falta a palavra obrigatória '{anchor}'."));
                    break;
                }
            }
            if row.error.is_none() {
                row.pool = pool.join(" ");
                pool.sort();
                let signature = pool.join(" ");
                if let Some(first) = seen.get(&signature) {
                    row.duplicate_of = Some(*first);
                } else {
                    seen.insert(signature, row.line);
                }
            }
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return Err("O arquivo não contém linhas com palavras.".into());
    }
    Ok(ImportPreview {
        import_id: super::unique_id(),
        filename: filename.to_owned(),
        valid_count: rows
            .iter()
            .filter(|r| r.error.is_none() && r.duplicate_of.is_none())
            .count(),
        invalid_count: rows.iter().filter(|r| r.error.is_some()).count(),
        duplicate_count: rows.iter().filter(|r| r.duplicate_of.is_some()).count(),
        rows,
    })
}

pub fn parse_file(path: &Path) -> Result<ImportPreview, String> {
    let mut file =
        File::open(path).map_err(|e| format!("Não foi possível abrir o arquivo: {e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_BYTES as u64 {
        return Err("O arquivo deve ter até 10 MB.".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("O arquivo deve ter até 10 MB.".into());
    }
    parse_bytes(
        &bytes,
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Arquivo"),
    )
}

pub fn parse_bytes(bytes: &[u8], filename: &str) -> Result<ImportPreview, String> {
    if bytes.len() > MAX_BYTES {
        return Err("O arquivo deve ter até 10 MB.".into());
    }
    let extension = Path::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let text=match extension.as_str() {
        "txt" => decode_txt(bytes)?,
        "doc"|"docx" => rwml::extract_text(bytes).map_err(|e|format!("Não foi possível ler o documento Word: {e}. Arquivos protegidos ou corrompidos precisam ser salvos como TXT."))?,
        _ => return Err("Selecione um arquivo TXT, DOC ou DOCX.".into()),
    };
    parse_text(&text, filename)
}
fn decode_txt(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !(bytes.len() - 2).is_multiple_of(2) {
            return Err("TXT UTF-16 incompleto.".into());
        }
        let little = bytes[0] == 0xff;
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                if little {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|_| "TXT UTF-16 inválido.".into())
    } else {
        String::from_utf8(bytes.to_vec())
            .map_err(|_| "Salve o TXT com codificação UTF-8 ou UTF-16.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const LINE: &str =
        "parrot abandon abandon dutch abandon abandon abandon fog abandon abandon abandon abandon";
    #[test]
    fn keeps_multiplicity_and_pins_independently_of_input_order() {
        let p = parse_text(LINE, "x.txt").unwrap();
        assert_eq!(p.valid_count, 1);
        assert_eq!(p.rows[0].pool.split_whitespace().count(), 9);
        assert!(p.rows[0].pool.split_whitespace().all(|w| w == "abandon"));
        let p = parse_text(&LINE.replacen("abandon", "dutch", 1), "x").unwrap();
        assert_eq!(
            p.rows[0]
                .pool
                .split_whitespace()
                .filter(|w| *w == "dutch")
                .count(),
            1
        );
    }
    #[test]
    fn reordered_duplicates_missing_anchors_and_bad_words_are_distinguished() {
        let reversed = LINE.split_whitespace().rev().collect::<Vec<_>>().join(" ");
        let text = format!(
            "\u{feff}{}\r\n\r\n{}\n{}\n{}\nwrong count",
            LINE.to_uppercase(),
            reversed,
            LINE.replace("fog", "cloud"),
            LINE.replace("fog", "culling")
        );
        let p = parse_text(&text, "x").unwrap();
        assert_eq!(
            (p.valid_count, p.duplicate_count, p.invalid_count),
            (1, 1, 3)
        );
        assert_eq!(p.rows[1].duplicate_of, Some(1));
        assert_eq!(p.rows[1].line, 3);
    }
    #[test]
    fn large_permutation_list_is_deduplicated_and_row_limit_remains_bounded() {
        let text = format!("{LINE}\n").repeat(22_615);
        let p = parse_text(&text, "large.txt").unwrap();
        assert_eq!(p.rows.len(), 22_615);
        assert_eq!((p.valid_count, p.duplicate_count, p.invalid_count), (1, 22_614, 0));
        assert_eq!(p.rows.last().unwrap().duplicate_of, Some(1));
        assert_eq!(p.rows.last().unwrap().line, 22_615);
        assert!(parse_text(&"x\n".repeat(MAX_ROWS), "limit.txt").is_ok());
        assert!(parse_text(&"x\n".repeat(MAX_ROWS + 1), "too-large.txt").is_err());
    }
    #[test]
    fn txt_encodings_and_empty_file() {
        let mut bytes = vec![0xff, 0xfe];
        for c in LINE.encode_utf16() {
            bytes.extend(c.to_le_bytes());
        }
        assert_eq!(decode_txt(&bytes).unwrap(), LINE);
        assert!(decode_txt(&[0xff]).is_err());
        assert!(parse_text("\n ", "empty").is_err());
        assert!(rwml::extract_text(b"not a Word document").is_err());
    }
}
