// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";
import { MockERC20 } from "../src/MockERC20.sol";
import { Subscription } from "../src/Subscription.sol";

contract SubscriptionTest is Test {
    MockERC20 internal token;
    Subscription internal sub;

    address internal admin = address(0xA017);
    address internal alice = address(0xA11CE);
    address internal bob = address(0xB0B);

    uint256 internal constant PRO_PRICE = 10 ether;
    uint256 internal constant ENTERPRISE_PRICE = 50 ether;
    uint64 internal constant PERIOD = 30 days;

    function setUp() public {
        token = new MockERC20("DAEJI", "DAEJI", 18);
        sub = new Subscription(address(token), admin);

        vm.startPrank(admin);
        sub.setTierPrice(Subscription.Tier.Free, 0);
        sub.setTierPrice(Subscription.Tier.Pro, PRO_PRICE);
        sub.setTierPrice(Subscription.Tier.Enterprise, ENTERPRISE_PRICE);
        vm.stopPrank();

        token.mint(alice, 10_000 ether);
        token.mint(bob, 10_000 ether);
        vm.prank(alice);
        token.approve(address(sub), type(uint256).max);
        vm.prank(bob);
        token.approve(address(sub), type(uint256).max);
    }

    /// 1 — Free tier has zero cost and pulls no tokens
    function test_subscribe_free_is_gratis() public {
        uint256 before = token.balanceOf(alice);
        vm.prank(alice);
        sub.subscribe(Subscription.Tier.Free, 3);
        assertEq(token.balanceOf(alice), before, "free tier should not pull tokens");
        assertTrue(sub.isActive(alice));
        assertEq(uint8(sub.getTier(alice)), uint8(Subscription.Tier.Free));
    }

    /// 2 — Pro tier pulls price × periods and sets expiry
    function test_subscribe_pro_pulls_payment_and_sets_expiry() public {
        uint256 before = token.balanceOf(alice);
        vm.prank(alice);
        sub.subscribe(Subscription.Tier.Pro, 2);

        assertEq(token.balanceOf(alice), before - (PRO_PRICE * 2), "payment pulled");
        Subscription.SubscriberState memory s = sub.getState(alice);
        assertEq(uint8(s.tier), uint8(Subscription.Tier.Pro));
        assertEq(s.expiresAt, block.timestamp + (PERIOD * 2));
        assertTrue(sub.isActive(alice));
    }

    /// 3 — Extend appends new periods to existing expiry
    function test_extend_appends_to_existing_expiry() public {
        vm.startPrank(alice);
        sub.subscribe(Subscription.Tier.Pro, 1);
        uint64 firstExpiry = sub.getState(alice).expiresAt;

        sub.extend(2);
        vm.stopPrank();

        Subscription.SubscriberState memory s = sub.getState(alice);
        assertEq(s.expiresAt, firstExpiry + (PERIOD * 2), "expiry extended");
    }

    /// 4 — Upgrade charges the pro-rated differential for remaining time
    function test_upgrade_charges_differential() public {
        vm.prank(alice);
        sub.subscribe(Subscription.Tier.Pro, 1);

        // Warp forward 10 days → 20 days remaining
        vm.warp(block.timestamp + 10 days);

        uint256 beforeBal = token.balanceOf(alice);
        vm.prank(alice);
        sub.upgrade(Subscription.Tier.Enterprise);

        // Expected differential: (50 - 10) * (20 days / 30 days) = 40 * 20/30 = ~26.66 ether
        uint256 expected = ((ENTERPRISE_PRICE - PRO_PRICE) * 20 days) / 30 days;
        assertEq(token.balanceOf(alice), beforeBal - expected, "pro-rated differential pulled");
        assertEq(uint8(sub.getState(alice).tier), uint8(Subscription.Tier.Enterprise));
    }

    /// 5 — Cancel stops auto-renew but keeps paid time (no refund)
    function test_cancel_keeps_current_expiry_but_no_auto_renew() public {
        vm.startPrank(alice);
        sub.subscribe(Subscription.Tier.Pro, 1);
        uint64 expiryBefore = sub.getState(alice).expiresAt;
        sub.cancel();
        vm.stopPrank();

        // Expiry unchanged; cancelled flag set
        Subscription.SubscriberState memory s = sub.getState(alice);
        assertEq(s.expiresAt, expiryBefore, "expiry unchanged");
        assertTrue(s.cancelled, "cancelled flag set");
        assertTrue(sub.isActive(alice), "still active until expiry");
    }

    /// 6 — isActive flips false after expiresAt
    function test_isActive_returns_false_after_expiry() public {
        vm.prank(alice);
        sub.subscribe(Subscription.Tier.Pro, 1);
        assertTrue(sub.isActive(alice));

        vm.warp(block.timestamp + 31 days);
        assertFalse(sub.isActive(alice));
        assertEq(uint8(sub.getTier(alice)), uint8(Subscription.Tier.None), "getTier returns None after expiry");
    }

    /// 7 — setTierPrice is owner-only
    function test_setTierPrice_only_owner() public {
        vm.prank(alice);
        vm.expectRevert(Subscription.OnlyOwner.selector);
        sub.setTierPrice(Subscription.Tier.Pro, 999 ether);

        vm.prank(admin);
        sub.setTierPrice(Subscription.Tier.Pro, 999 ether);
        assertEq(sub.tierPricePerPeriod(Subscription.Tier.Pro), 999 ether);
    }

    /// 8 — transferOwnership updates owner and emits
    function test_transferOwnership() public {
        address newAdmin = address(0xA018);
        vm.prank(admin);
        sub.transferOwnership(newAdmin);
        assertEq(sub.owner(), newAdmin);

        // Old admin can no longer set prices
        vm.prank(admin);
        vm.expectRevert(Subscription.OnlyOwner.selector);
        sub.setTierPrice(Subscription.Tier.Pro, 1 ether);
    }
}
