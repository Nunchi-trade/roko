// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";
import { MockERC20 } from "../src/MockERC20.sol";
import { ComputeBond } from "../src/ComputeBond.sol";

contract ComputeBondTest is Test {
    MockERC20 internal token;
    ComputeBond internal bondContract;

    address internal admin = address(0xA017);
    address internal provider = address(0xD474);
    address internal slasher = address(0x5145);
    address internal stranger = address(0xDEAD);

    // Cached constants — avoids external calls to bondContract.MIN_BOND() etc.
    // inside argument expressions, which would otherwise consume `vm.prank` /
    // `vm.expectRevert` before the intended call.
    uint256 internal MIN_BOND_AMOUNT;
    uint256 internal SILVER_AMOUNT;
    uint256 internal PLATINUM_AMOUNT;
    uint8 internal FRAUD;
    uint8 internal STALE_DATA;
    uint64 internal UNBOND_DELAY;

    function setUp() public {
        token = new MockERC20("DAEJI", "DAEJI", 18);
        bondContract = new ComputeBond(address(token), admin);

        MIN_BOND_AMOUNT = bondContract.MIN_BOND();
        SILVER_AMOUNT = bondContract.SILVER_THRESHOLD();
        PLATINUM_AMOUNT = bondContract.PLATINUM_THRESHOLD();
        FRAUD = bondContract.SLASH_FRAUD();
        STALE_DATA = bondContract.SLASH_STALE_DATA();
        UNBOND_DELAY = bondContract.UNBOND_DELAY();

        vm.prank(admin);
        bondContract.setAuthorized(slasher, true);

        token.mint(provider, 2_000_000 ether);
        vm.prank(provider);
        token.approve(address(bondContract), type(uint256).max);
    }

    function _register(uint256 amount) internal {
        vm.prank(provider);
        bondContract.register(amount);
    }

    /// 1 — register pulls stake and starts at Bronze tier
    function test_register_pulls_stake_and_starts_at_bronze() public {
        uint256 before = token.balanceOf(provider);
        _register(MIN_BOND_AMOUNT);
        assertEq(token.balanceOf(provider), before - MIN_BOND_AMOUNT);
        assertEq(uint8(bondContract.tierOf(provider)), uint8(ComputeBond.ProviderTier.Bronze));
        assertTrue(bondContract.isActive(provider));
    }

    /// 2 — register reverts below MIN_BOND
    function test_register_reverts_below_min_bond() public {
        uint256 tooLow = MIN_BOND_AMOUNT - 1;
        vm.prank(provider);
        vm.expectRevert(ComputeBond.InsufficientBond.selector);
        bondContract.register(tooLow);
    }

    /// 3 — additional bond calls add to existing and can cross tier thresholds
    function test_bond_adds_to_existing_and_crosses_tier() public {
        _register(MIN_BOND_AMOUNT);
        assertEq(uint8(bondContract.tierOf(provider)), uint8(ComputeBond.ProviderTier.Bronze));

        // Cross into Silver
        uint256 topUp = SILVER_AMOUNT - MIN_BOND_AMOUNT;
        vm.prank(provider);
        bondContract.bond(topUp);
        assertEq(uint8(bondContract.tierOf(provider)), uint8(ComputeBond.ProviderTier.Silver));
    }

    /// 4 — requestUnbond starts the withdraw timer
    function test_requestUnbond_starts_timer() public {
        _register(MIN_BOND_AMOUNT);
        vm.prank(provider);
        bondContract.requestUnbond();
        assertEq(bondContract.getProvider(provider).unbondRequestedAt, uint64(block.timestamp));
    }

    /// 5 — withdraw reverts before unbond delay has elapsed
    function test_withdraw_reverts_before_delay() public {
        _register(MIN_BOND_AMOUNT);
        vm.prank(provider);
        bondContract.requestUnbond();
        vm.prank(provider);
        vm.expectRevert(ComputeBond.UnbondTooEarly.selector);
        bondContract.withdraw();
    }

    /// 6 — withdraw after delay transfers full bond back
    function test_withdraw_after_delay_transfers_stake() public {
        _register(MIN_BOND_AMOUNT);
        vm.prank(provider);
        bondContract.requestUnbond();

        vm.warp(block.timestamp + UNBOND_DELAY + 1);
        uint256 before = token.balanceOf(provider);
        vm.prank(provider);
        bondContract.withdraw();
        assertEq(token.balanceOf(provider), before + MIN_BOND_AMOUNT);
        assertEq(bondContract.getProvider(provider).bond, 0);
    }

    /// 7 — slash(FRAUD) burns the full bond
    function test_slash_fraud_burns_full_bond() public {
        _register(MIN_BOND_AMOUNT);
        vm.prank(slasher);
        bondContract.slash(provider, FRAUD);
        assertEq(bondContract.getProvider(provider).bond, 0, "fraud slashes 100%");
    }

    /// 8 — slash(STALE_DATA) burns 5% of current bond
    function test_slash_stale_data_burns_5_percent() public {
        _register(MIN_BOND_AMOUNT);
        uint256 expectedRemaining = MIN_BOND_AMOUNT - ((MIN_BOND_AMOUNT * 500) / 10_000);
        vm.prank(slasher);
        bondContract.slash(provider, STALE_DATA);
        assertEq(bondContract.getProvider(provider).bond, expectedRemaining);
    }

    /// 9 — slash() only authorized
    function test_slash_only_authorized() public {
        _register(MIN_BOND_AMOUNT);
        vm.prank(stranger);
        vm.expectRevert(ComputeBond.NotAuthorizedCaller.selector);
        bondContract.slash(provider, STALE_DATA);
    }

    /// 10 — tierOf reflects bond thresholds
    function test_tierOf_reflects_bond_thresholds() public {
        // Register at Platinum threshold directly
        vm.prank(provider);
        bondContract.register(PLATINUM_AMOUNT);
        assertEq(uint8(bondContract.tierOf(provider)), uint8(ComputeBond.ProviderTier.Platinum));
    }
}
