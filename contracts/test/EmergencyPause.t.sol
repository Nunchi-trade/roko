// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";
import { EmergencyPause } from "../src/EmergencyPause.sol";

contract EmergencyPauseTest is Test {
    EmergencyPause internal pauser;

    address internal admin = address(0xA017);
    address internal stranger = address(0xDEAD);

    bytes32 internal constant BOUNTY_MARKET = keccak256("bounty-market");
    bytes32 internal constant INSIGHT_BOARD = keccak256("insight-board");

    event GlobalPaused(address indexed actor, string reason);
    event GlobalUnpaused(address indexed actor);
    event CategoryPaused(bytes32 indexed category, address indexed actor, string reason);

    function setUp() public {
        pauser = new EmergencyPause(admin);
    }

    /// 1 — pauseGlobal sets the flag and emits
    function test_pauseGlobal_sets_flag_and_emits() public {
        vm.expectEmit(true, false, false, true, address(pauser));
        emit GlobalPaused(admin, "demo failure");

        vm.prank(admin);
        pauser.pauseGlobal("demo failure");

        assertTrue(pauser.isGloballyPaused());
        assertTrue(pauser.isPaused(BOUNTY_MARKET), "global pause affects any category");
    }

    /// 2 — unpauseGlobal clears the flag
    function test_unpauseGlobal_clears_flag() public {
        vm.startPrank(admin);
        pauser.pauseGlobal("x");
        pauser.unpauseGlobal();
        vm.stopPrank();

        assertFalse(pauser.isGloballyPaused());
        assertFalse(pauser.isPaused(BOUNTY_MARKET));
    }

    /// 3 — category pause is independent of global
    function test_pauseCategory_independent_of_global() public {
        vm.prank(admin);
        pauser.pauseCategory(BOUNTY_MARKET, "stuck job");

        assertFalse(pauser.isGloballyPaused(), "global untouched");
        assertTrue(pauser.isCategoryPaused(BOUNTY_MARKET));
        assertTrue(pauser.isPaused(BOUNTY_MARKET));
        assertFalse(pauser.isPaused(INSIGHT_BOARD), "different category unaffected");
    }

    /// 4 — isPaused returns true if either global OR category is paused
    function test_isPaused_true_if_global_or_category() public {
        // Both pathways lead to the same result
        vm.prank(admin);
        pauser.pauseCategory(INSIGHT_BOARD, "indexer lag");
        assertTrue(pauser.isPaused(INSIGHT_BOARD));

        vm.prank(admin);
        pauser.pauseGlobal("worse failure");
        assertTrue(pauser.isPaused(INSIGHT_BOARD));
        assertTrue(pauser.isPaused(BOUNTY_MARKET));
    }

    /// 5 — pause operations are owner-only
    function test_pause_only_owner() public {
        vm.prank(stranger);
        vm.expectRevert(EmergencyPause.OnlyOwner.selector);
        pauser.pauseGlobal("attempt");

        vm.prank(stranger);
        vm.expectRevert(EmergencyPause.OnlyOwner.selector);
        pauser.pauseCategory(BOUNTY_MARKET, "attempt");
    }
}
