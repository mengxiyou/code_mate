use serde_json::{json, Value};

fn theme(primary: u32, meter_a: u32, meter_b: u32, meter_c: u32) -> Value {
    json!({
        "primary": primary,
        "meter_a": meter_a,
        "meter_b": meter_b,
        "meter_c": meter_c,
    })
}

pub fn for_provider(provider: &str) -> Value {
    match provider {
        // Keep Claude's existing visual language unchanged.
        "ClaudeCode" => theme(0xF08A5E, 0xFF7A4D, 0x4DECEF, 0xFFB44E),
        "Codex" => theme(0x19C37D, 0xE7C66A, 0x62D6FF, 0x19C37D),
        "System" => system(),
        _ => theme(0xF08A5E, 0xFF7A4D, 0x4DECEF, 0xFFB44E),
    }
}

pub fn system() -> Value {
    theme(0x6DCBFF, 0x2DE2E6, 0x8EA7FF, 0x6DCBFF)
}
