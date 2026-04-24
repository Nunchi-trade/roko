#!/usr/bin/env bash
# 01-provider-healthcheck.sh — Verify all configured providers are reachable
source "$(dirname "$0")/common.sh"

banner
step 1 "Provider Health Check"
narrate "Verifying every LLM provider is reachable and responsive"

WORKSPACE=$(setup_workspace)
trap "cleanup_workspace '$WORKSPACE'" EXIT
cd "$WORKSPACE"

info "Workspace: $WORKSPACE"
info "Config:    $DEMO_CONFIG"
echo

# Initialize roko workspace
"$ROKO" init 2>/dev/null || true

hr
echo -e "  ${BOLD}Configured Providers${NC}"
hr
echo
"$ROKO" config providers list --workdir "$WORKSPACE"
echo

hr
echo -e "  ${BOLD}Connectivity Tests${NC}"
hr
echo

# Run provider tests and capture output for table formatting
if "$ROKO" config providers test --all --workdir "$WORKSPACE"; then
    echo
    ok "${GREEN}${BOLD}All available providers passed${NC}"
else
    echo
    warn "Some providers failed — check output above"
fi

echo
hr
echo -e "  ${DIM}${ITALIC}Provider health verified.${NC}"
echo
