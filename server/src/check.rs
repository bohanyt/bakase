//! `rag-hub-server cek-vision`: memastikan deployment Azure bisa membaca gambar dan
//! memanggil tool. Hanya memakai gambar dan tool tiruan, tanpa data kerja.
//!
//! model adalah model reasoning. Lewat Chat Completions, tool hanya bisa dipakai kalau
//! reasoning dimatikan, jadi jalur utama harness adalah Responses API.

use crate::agent::{function_calls, output_text};
use crate::llm::Llm;
use base64::Engine;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Gambar uji: tulisan "TES-4821" dan lingkaran merah.
const TEST_IMAGE: &[u8] = include_bytes!("../assets/cek-vision.png");
const TEST_CODE: &str = "4821";
const TEST_STOCK: &str = "120";

pub async fn run(llm: &Llm) -> anyhow::Result<()> {
    let mut failed = 0;
    let mut report = |label: &str, ok: bool, detail: String| {
        if !ok {
            failed += 1;
        }
        println!(
            "{label:<36}: {} · {detail}",
            if ok { "BISA" } else { "TIDAK" }
        );
    };

    // 1. Baca gambar lewat Responses API.
    let image = base64::engine::general_purpose::STANDARD.encode(TEST_IMAGE);
    let input = json!([{ "role": "user", "content": [
        { "type": "input_text", "text": "Tulis kode yang tertulis di gambar ini dan sebutkan bentuk berwarna merah di dalamnya. Jawab singkat." },
        { "type": "input_image", "image_url": format!("data:image/png;base64,{image}") },
    ]}]);
    let started = Instant::now();
    match llm.respond(input, json!({})).await {
        Ok(r) => {
            let text = output_text(&r);
            let tokens = r["usage"]["input_tokens"].as_u64().unwrap_or(0);
            report(
                "Baca gambar (Responses)",
                text.contains(TEST_CODE),
                format!(
                    "{}, {tokens} token masuk · \"{}\"",
                    secs(started.elapsed()),
                    short(&text)
                ),
            );
        }
        Err(e) => report("Baca gambar (Responses)", false, format!("{e:#}")),
    }

    // 1b. Baca PDF dua halaman lewat `input_file`, untuk dokumen hasil scan di ingest.
    let pdf = base64::engine::general_purpose::STANDARD.encode(crate::ingest::minimal_pdf(&[
        "Halaman satu: kode uji PDF-7319.",
        "Halaman dua: stok 45 batang.",
    ]));
    let input = json!([{ "role": "user", "content": [
        { "type": "input_text", "text": "Sebutkan kode uji di halaman satu dan angka stok di halaman dua. Jawab singkat." },
        { "type": "input_file", "filename": "cek-vision.pdf", "file_data": format!("data:application/pdf;base64,{pdf}") },
    ]}]);
    let started = Instant::now();
    match llm.respond(input, json!({})).await {
        Ok(r) => {
            let text = output_text(&r);
            let tokens = r["usage"]["input_tokens"].as_u64().unwrap_or(0);
            report(
                "Baca PDF (Responses, input_file)",
                text.contains("7319") && text.contains("45"),
                format!(
                    "{}, {tokens} token masuk · \"{}\"",
                    secs(started.elapsed()),
                    short(&text)
                ),
            );
        }
        Err(e) => report("Baca PDF (Responses, input_file)", false, format!("{e:#}")),
    }

    // 2–3. Panggil tool dengan reasoning aktif, lalu kirim hasil tool tanpa state di Azure.
    let tool = json!({
        "type": "function",
        "name": "cek_stok_produk",
        "description": "Cek stok produk contoh di gudang berdasarkan ukuran.",
        "parameters": {
            "type": "object",
            "properties": { "ukuran": { "type": "string", "description": "Ukuran produk, misalnya 2 inch" } },
            "required": ["ukuran"],
        },
    });
    let question = json!({ "role": "user", "content": "Berapa stok produk ukuran 2 inch di gudang sekarang?" });
    let started = Instant::now();
    // Tool dipaksa di giliran pertama, supaya yang diuji kemampuan deployment, bukan
    // keputusan model; panggilan paralel dimatikan.
    let first = llm
        .respond(
            json!([question]),
            json!({
                "tools": [tool],
                "tool_choice": { "type": "function", "name": "cek_stok_produk" },
                "parallel_tool_calls": false,
            }),
        )
        .await;
    match first {
        Ok(r) => match function_calls(&r).into_iter().next() {
            Some(call) => {
                report(
                    "Panggil tool (Responses, reasoning)",
                    true,
                    format!(
                        "{} · {}({})",
                        secs(started.elapsed()),
                        call.name,
                        short(&call.arguments)
                    ),
                );
                // Giliran berikutnya membawa semua item keluaran (termasuk reasoning
                // terenkripsi) ditambah hasil untuk setiap panggilan tool. Tool tidak
                // boleh dipanggil lagi, jadi model harus menjawab dengan teks.
                let mut input = vec![question.clone()];
                input.extend(r["output"].as_array().cloned().unwrap_or_default());
                for c in function_calls(&r) {
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": c.id,
                        "output": format!("{{\"stok\": {TEST_STOCK}, \"satuan\": \"batang\"}}"),
                    }));
                }
                let started = Instant::now();
                match llm
                    .respond(
                        Value::Array(input),
                        json!({ "tools": [tool], "tool_choice": "none" }),
                    )
                    .await
                {
                    Ok(r) => {
                        let text = output_text(&r);
                        report(
                            "Lanjutan tool (Responses)",
                            text.contains(TEST_STOCK),
                            format!("{} · \"{}\"", secs(started.elapsed()), short(&text)),
                        );
                    }
                    Err(e) => report("Lanjutan tool (Responses)", false, format!("{e:#}")),
                }
            }
            None => report(
                "Panggil tool (Responses, reasoning)",
                false,
                format!("model menjawab langsung · \"{}\"", short(&output_text(&r))),
            ),
        },
        Err(e) => report(
            "Panggil tool (Responses, reasoning)",
            false,
            format!("{e:#}"),
        ),
    }

    // Info saja, tidak dihitung: Chat Completions bisa memakai tool kalau reasoning dimatikan.
    let chat_tool = json!([{ "type": "function", "function": {
        "name": tool["name"], "description": tool["description"], "parameters": tool["parameters"],
    }}]);
    let started = Instant::now();
    let info = match llm
        .complete(
            json!([question]),
            json!({ "tools": chat_tool, "reasoning_effort": "none" }),
        )
        .await
    {
        Ok(r) if r["choices"][0]["message"]["tool_calls"][0]["function"]["name"].is_string() => {
            format!("BISA · {}", secs(started.elapsed()))
        }
        Ok(_) => "TIDAK · model menjawab langsung".to_string(),
        Err(e) => format!("TIDAK · {e:#}"),
    };
    println!(
        "{:<36}: {info} (info, tanpa reasoning)",
        "Panggil tool (Chat Completions)"
    );

    // Info saja: prompt caching Azure. Awalan panjang yang sama dikirim dua kali; giliran
    // kedua seharusnya mengambil sebagian besar token dari cache.
    let filler =
        "produk contoh tersedia dalam berbagai ukuran dan ketebalan untuk konstruksi dan industri. "
            .repeat(90);
    let long = json!([{ "role": "user", "content": format!("{filler}\nJawab satu kata: apa bahan produknya?") }]);
    let info = match llm.respond(long.clone(), json!({})).await {
        Ok(_) => match llm.respond(long, json!({})).await {
            Ok(r) => {
                let total = r["usage"]["input_tokens"].as_u64().unwrap_or(0);
                let cached = r["usage"]["input_tokens_details"]["cached_tokens"]
                    .as_u64()
                    .unwrap_or(0);
                let verdict = if cached > 0 { "BISA" } else { "TIDAK" };
                format!("{verdict} · {cached} dari {total} token masuk diambil dari cache")
            }
            Err(e) => format!("TIDAK · {e:#}"),
        },
        Err(e) => format!("TIDAK · {e:#}"),
    };
    println!("{:<36}: {info} (info)", "Cache prompt (Responses)");

    anyhow::ensure!(failed == 0, "{failed} cek tidak lulus");
    Ok(())
}

fn secs(d: Duration) -> String {
    format!("{:.1} dtk", d.as_secs_f64()).replace('.', ",")
}

fn short(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 140 {
        format!("{}…", one_line.chars().take(140).collect::<String>())
    } else {
        one_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_and_seconds() {
        assert_eq!(short("a\n b   c"), "a b c");
        assert!(short(&"x".repeat(200)).ends_with('…'));
        assert_eq!(secs(Duration::from_millis(2340)), "2,3 dtk");
    }
}
