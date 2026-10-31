use std::path::Path;

fn main() {
    let svg_path = Path::new("assets/camcap-icon.svg");
    let png_path = Path::new("assets/camcap-256.png");
    let ico_path = Path::new("assets/camcap.ico");

    println!("cargo:rerun-if-changed={}", svg_path.display());

    // 1. 解析 SVG
    let svg_data = std::fs::read(svg_path).expect("读取 SVG 失败");
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(&svg_data, &opt)
        .expect("解析 SVG 失败");

    // 2. 生成 256×256 PNG（给 eframe 窗口图标用）
    write_png(&tree, 256, png_path);

    // 3. 生成多尺寸 ICO（给 Windows exe 图标用）
    let sizes: &[u32] = &[16, 24, 32, 48, 64, 128, 256];
    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);
    for &s in sizes {
        let rgba = render_rgba(&tree, s);
        let image = ico::IconImage::from_rgba_data(s, s, rgba);
        let entry = ico::IconDirEntry::encode(&image)
            .expect("编码 ICO 帧失败");
        icon_dir.add_entry(entry);
    }
    icon_dir
        .write(std::fs::File::create(ico_path).expect("创建 ICO 失败"))
        .expect("写 ICO 失败");

    // 4. Windows 下把 ICO 嵌入到 exe 资源里
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_str().expect("ICO 路径非 UTF-8"));
        res.compile().expect("嵌入 exe 图标失败");
    }
}

/// 用 resvg 把 SVG 渲染成 size×size 的 RGBA 像素
fn render_rgba(tree: &resvg::usvg::Tree, size: u32) -> Vec<u8> {
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(size, size).expect("创建 pixmap 失败");
    let scale = size as f32 / tree.size().width();
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia 的像素是预乘 alpha，转成直通 alpha 供 ICO/PNG 使用
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

fn write_png(tree: &resvg::usvg::Tree, size: u32, path: &Path) {
    let rgba = render_rgba(tree, size);
    let img = image::RgbaImage::from_raw(size, size, rgba)
        .expect("创建 image 失败");
    img.save(path).expect("保存 PNG 失败");
}