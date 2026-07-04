//! 构建脚本:把品牌环 "O" 渲染成多尺寸 .ico 并嵌进 exe 资源(Windows 文件图标 / 任务栏)。
//! 纯 Rust 手写 ICO(32bpp BMP 条目,无外部 imaging 依赖);winresource 负责定位 SDK rc.exe 嵌入。
//! 几何与 ui.rs 的 ring_rgba / 固件 loading 屏的 "O" 一致(此处独立一份:build.rs 不能引用 crate src)。

#[cfg(windows)]
fn main() {
    use std::path::Path;
    println!("cargo:rerun-if-changed=build.rs");

    // 仅 Windows 目标嵌图标(从 win 主机交叉编到非 win 时跳过)
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let ico_path = Path::new(&out_dir).join("code_mate.ico");
    let ico = build_ico(&[16, 24, 32, 48, 64], 0xFF, 0xB4, 0x4E);
    std::fs::write(&ico_path, &ico).expect("write code_mate.ico");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico_path.to_str().expect("ico path utf8"));
    if let Err(e) = res.compile() {
        // 缺 rc.exe 等环境问题不致命:打 warning,exe 退化为默认图标(仍可构建)。
        println!("cargo:warning=嵌入 exe 图标失败(将用默认图标): {e}");
    }
}

#[cfg(not(windows))]
fn main() {}

// ---------- 纯 Rust 画环 + 手写 ICO(无依赖)----------

#[cfg(windows)]
fn ring_rgba(s: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut px = vec![0u8; (s * s * 4) as usize];
    let cf = s as f32 / 2.0;
    let r_out = cf * 0.90; // 外半径留 ~10% 边距防裁切
    let stroke = s as f32 * 0.16;
    let r_mid = r_out - stroke / 2.0; // 环中线半径
    let half = stroke / 2.0;
    let cap = |deg: f32| {
        let a = deg.to_radians();
        (cf + r_mid * a.cos(), cf + r_mid * a.sin())
    };
    let (p1x, p1y) = cap(45.0); // 缺口右沿圆头
    let (p2x, p2y) = cap(135.0); // 缺口左沿圆头
    const SS: u32 = 4; // 4×4 超采样抗锯齿
    for y in 0..s {
        for x in 0..s {
            let mut cov = 0.0f32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let fy = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    let (dx, dy) = (fx - cf, fy - cf);
                    let d = (dx * dx + dy * dy).sqrt();
                    let mut inside = false;
                    if (d - r_mid).abs() <= half {
                        let ang = dy.atan2(dx).to_degrees(); // 下=+90°
                        if !(ang > 45.0 && ang < 135.0) {
                            inside = true; // 环身(排除底部缺口)
                        }
                    }
                    if !inside {
                        let d1 = ((fx - p1x).powi(2) + (fy - p1y).powi(2)).sqrt();
                        let d2 = ((fx - p2x).powi(2) + (fy - p2y).powi(2)).sqrt();
                        inside = d1 <= half || d2 <= half; // 两端圆头
                    }
                    if inside {
                        cov += 1.0;
                    }
                }
            }
            cov /= (SS * SS) as f32;
            let i = ((y * s + x) * 4) as usize;
            px[i] = r;
            px[i + 1] = g;
            px[i + 2] = b;
            px[i + 3] = (cov * 255.0).round() as u8;
        }
    }
    px
}

// 单尺寸 → ICO 内嵌 BMP(BITMAPINFOHEADER + 32bpp BGRA 自底向上 + 全 0 AND 掩码)
#[cfg(windows)]
fn rgba_to_dib(s: u32, rgba: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&40u32.to_le_bytes()); // biSize
    b.extend_from_slice(&(s as i32).to_le_bytes()); // biWidth
    b.extend_from_slice(&((2 * s) as i32).to_le_bytes()); // biHeight = XOR+AND 双高
    b.extend_from_slice(&1u16.to_le_bytes()); // planes
    b.extend_from_slice(&32u16.to_le_bytes()); // bpp
    b.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    b.extend_from_slice(&0u32.to_le_bytes()); // sizeImage
    b.extend_from_slice(&0i32.to_le_bytes()); // xppm
    b.extend_from_slice(&0i32.to_le_bytes()); // yppm
    b.extend_from_slice(&0u32.to_le_bytes()); // clrUsed
    b.extend_from_slice(&0u32.to_le_bytes()); // clrImportant
    for y in (0..s).rev() {
        // 自底向上
        for x in 0..s {
            let i = ((y * s + x) * 4) as usize;
            b.push(rgba[i + 2]); // B
            b.push(rgba[i + 1]); // G
            b.push(rgba[i]); // R
            b.push(rgba[i + 3]); // A(直 alpha;掩码留全 0,靠 alpha 透明)
        }
    }
    let mask_row = (((s + 31) / 32) * 4) as usize; // 1bpp AND 掩码,行按 4 字节对齐
    b.extend(std::iter::repeat(0u8).take(mask_row * s as usize));
    b
}

#[cfg(windows)]
fn build_ico(sizes: &[u32], r: u8, g: u8, b: u8) -> Vec<u8> {
    let imgs: Vec<(u32, Vec<u8>)> =
        sizes.iter().map(|&s| (s, rgba_to_dib(s, &ring_rgba(s, r, g, b)))).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type = icon
    out.extend_from_slice(&(imgs.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * imgs.len();
    for (s, dib) in &imgs {
        let wh = if *s >= 256 { 0u8 } else { *s as u8 };
        out.push(wh); // width(0=256)
        out.push(wh); // height
        out.push(0); // 调色板色数
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(dib.len() as u32).to_le_bytes()); // 数据字节数
        out.extend_from_slice(&(offset as u32).to_le_bytes()); // 偏移
        offset += dib.len();
    }
    for (_s, dib) in &imgs {
        out.extend_from_slice(dib);
    }
    out
}
