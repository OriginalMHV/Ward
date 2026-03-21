use anyhow::Result;
use rust_embed::Embed;
use tera::Tera;

#[derive(Embed)]
#[folder = "templates/"]
pub struct TemplateAssets;

/// Load embedded templates into a Tera instance.
pub fn load_templates() -> Result<Tera> {
    let mut tera = Tera::default();

    for file in TemplateAssets::iter() {
        let path = file.as_ref();
        if let Some(content) = TemplateAssets::get(path) {
            let text = std::str::from_utf8(content.data.as_ref())?;
            tera.add_raw_template(path, text)?;
        }
    }

    Ok(tera)
}
