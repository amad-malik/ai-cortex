// Another type whose name ends in Ctx is not the Ctx.
pub fn other() { let _ = ConnCtx::new(1); let _ = MyCtx::with_route_table(2); }
