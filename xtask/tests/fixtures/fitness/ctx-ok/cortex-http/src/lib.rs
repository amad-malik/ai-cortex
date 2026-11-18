pub fn guard_step(p: Principal, id: RequestId, rule: TargetRule, policy: Arc<dyn ScopePolicy>) -> Ctx {
    Ctx::new(p, id).with_route(rule, policy)
}
