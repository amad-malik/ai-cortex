// A piece never makes a Ctx: Ctx::new( and .with_route( are the chain's.
/* block: Ctx::new(p, id).with_route(r, pol) */
pub const A: &str = "Ctx::new(p, id).with_route(r, pol)";
pub const B: &str = r#"let c = Ctx::new(p, id);"#;
pub fn uses(ctx: &Ctx) -> String {
    ctx.clone().with_path([("id".to_owned(), "x".to_owned())]);
    format!("{}", ctx.request_id())
}
