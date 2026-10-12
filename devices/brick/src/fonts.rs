//! Works around TrimUI's stock system, which ships fontconfig but no fonts. Slint's text layout
//! (fontique) asks fontconfig for system fonts at start-up and panics when fontconfig finds none.
//! Without fontconfig (our own system image) it simply uses the fonts Specular embeds.

use std::path::Path;

const DIR: &str = "/tmp/specular-fonts";

/// Points fontconfig at one of Specular's fonts, if fontconfig is installed (at
/// /usr/lib/libfontconfig.so.1) and FONTCONFIG_FILE isn't already set. Writes the font and a
/// config file under /tmp/specular-fonts. Must run before any other thread starts, since it sets
/// an environment variable.
pub fn work_around_empty_fontconfig() -> std::io::Result<()> {
    if std::env::var_os("FONTCONFIG_FILE").is_some() || !Path::new("/usr/lib/libfontconfig.so.1").exists() {
        return Ok(());
    }
    std::fs::create_dir_all(DIR)?;
    std::fs::write(format!("{DIR}/Geist.ttf"), include_bytes!("../../../assets/fonts/Geist.ttf"))?;
    let conf = format!("{DIR}/fonts.conf");
    std::fs::write(&conf, format!("<?xml version=\"1.0\"?>\n<fontconfig><dir>{DIR}</dir><cachedir>{DIR}/cache</cachedir></fontconfig>\n"))?;
    // SAFETY: called at start-up, before any other thread exists.
    unsafe { std::env::set_var("FONTCONFIG_FILE", conf) };
    Ok(())
}
