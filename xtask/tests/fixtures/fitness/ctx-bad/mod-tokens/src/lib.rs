// Ctx::new( in a comment is not a call
pub fn forge(p: Principal, id: RequestId) -> Ctx {
    let c = Ctx::new(p, id);
    c
}
