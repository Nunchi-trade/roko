// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";

import { IdentityRegistry } from "../src/IdentityRegistry.sol";
import { MockERC20 } from "../src/MockERC20.sol";

contract IdentityRegistryTest is Test {
    MockERC20 internal token;
    IdentityRegistry internal registry;

    address internal admin = address(this);
    address internal alice = address(0xA11CE);
    address internal registrar = address(0xBEEF);

    function setUp() public {
        token = new MockERC20("KORAI", "KORAI", 18);
        registry = new IdentityRegistry(admin, address(token));

        token.mint(alice, 100_000 ether);
        vm.prank(alice);
        token.approve(address(registry), type(uint256).max);
    }

    function test_registerPassport_mintsSoulboundPassport() public {
        vm.prank(alice);
        uint256 passportId = registry.registerPassport(alice, 7, keccak256("prompt"), bytes32(0), 0);

        assertEq(passportId, 1);
        assertEq(registry.ownerOf(passportId), alice);
        assertEq(registry.ownerToPassportId(alice), passportId);
        assertEq(registry.balanceOf(alice), 1);
        assertTrue(registry.locked(passportId));

        vm.expectRevert(IdentityRegistry.Soulbound.selector);
        vm.prank(alice);
        registry.transferFrom(alice, address(0xCAFE), passportId);
    }

    function test_register_and_updateAgentCardUri_usesRequiredAbi() public {
        assertEq(
            IdentityRegistry.updateAgentCardUri.selector,
            bytes4(keccak256("updateAgentCardUri(uint256,string)"))
        );
        assertTrue(
            IdentityRegistry.updateAgentCardUri.selector
                != bytes4(keccak256("updateAgentCardUri(string,string)"))
        );

        registry.setRegistrar(registrar, true);

        vm.prank(registrar);
        uint256 passportId =
            registry.register(alice, 1 << 5, registry.TIER_WORKER(), keccak256("prompt"), "ipfs://card-v1");

        vm.prank(alice);
        registry.updateAgentCardUri(passportId, "ipfs://card-v2");

        IdentityRegistry.AgentPassport memory passport = registry.getPassport(passportId);
        assertEq(passport.agentCardUri, "ipfs://card-v2");
        assertEq(registry.tokenURI(passportId), "ipfs://card-v2");
    }

    function test_promptTimelock_and_domainStakingAdjustTier() public {
        vm.prank(alice);
        uint256 passportId = registry.registerPassport(alice, 0, keccak256("prompt-v1"), bytes32(0), 0);

        vm.prank(alice);
        registry.updatePromptHash(passportId, keccak256("prompt-v2"));

        vm.prank(alice);
        vm.expectRevert(IdentityRegistry.PromptUpdateNotReady.selector);
        registry.updatePromptHash(passportId, keccak256("prompt-v2"));

        vm.warp(block.timestamp + registry.PROMPT_UPDATE_DELAY());
        vm.prank(alice);
        registry.updatePromptHash(passportId, keccak256("prompt-v2"));

        IdentityRegistry.AgentPassport memory passport = registry.getPassport(passportId);
        assertEq(passport.systemPromptHash, keccak256("prompt-v2"));
        assertEq(registry.getTier(passportId), registry.TIER_EDGE());

        vm.prank(alice);
        registry.stakeIntoDomain(passportId, "solidity", 5_000 ether);
        assertEq(registry.getTier(passportId), registry.TIER_WORKER());

        vm.prank(alice);
        registry.stakeIntoDomain(passportId, "security", 20_000 ether);
        assertEq(registry.getTier(passportId), registry.TIER_SOVEREIGN());

        vm.expectRevert(abi.encodeWithSelector(IdentityRegistry.CooldownActive.selector, uint64(block.timestamp + registry.WITHDRAW_COOLDOWN())));
        vm.prank(alice);
        registry.withdrawFromDomain(passportId, "security", 1 ether);

        vm.warp(block.timestamp + registry.WITHDRAW_COOLDOWN());
        vm.prank(alice);
        registry.withdrawFromDomain(passportId, "security", 20_000 ether);
        assertEq(registry.getTier(passportId), registry.TIER_WORKER());
    }
}
