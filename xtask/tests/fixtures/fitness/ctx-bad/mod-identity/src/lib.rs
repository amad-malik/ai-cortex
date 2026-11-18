pub fn replace(ctx: &Ctx, rule: TargetRule, policy: Arc<dyn ScopePolicy>) -> Ctx {
    ctx.clone().with_route(rule, policy)
}
