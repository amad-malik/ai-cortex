impl Ctx {
    pub fn new(principal: Principal, request_id: RequestId) -> Ctx { todo!() }
    pub fn with_route(self, rule: TargetRule, policy: Arc<dyn ScopePolicy>) -> Ctx { todo!() }
}
#[cfg(test)]
mod tests {
    fn own_tests() { let _ = Ctx::new(p, id).with_route(r, pol); }
}
