//! Extract user-supplied compiled XNB effects for static shader analysis.
use magicka_content::{assets::Asset, parse_primary, xna::XnaObject, Content};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err("usage: effect-export input.xnb output.fxo".into());
    }
    let bytes = std::fs::read(&args[1])?;
    let xnb = magicka_xnb::xnb::parse(&bytes)?;
    let parsed = parse_primary(&xnb).ok_or("null asset")??;
    let Content::Xna(object) = parsed.content else {
        return Err("expected XNA effect".into());
    };
    let XnaObject::Asset(asset) = *object else {
        return Err("expected asset".into());
    };
    let Asset::Effect(code) = *asset else {
        return Err("expected compiled effect".into());
    };
    std::fs::write(&args[2], code)?;
    Ok(())
}
