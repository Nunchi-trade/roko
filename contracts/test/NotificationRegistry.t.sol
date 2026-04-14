// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";
import { NotificationRegistry } from "../src/NotificationRegistry.sol";

contract NotificationRegistryTest is Test {
    NotificationRegistry internal registry;
    address internal alice = address(0xA11CE);
    address internal bob = address(0xB0B);

    event PreferenceAdded(address indexed subscriber, uint256 indexed index, NotificationRegistry.Kind kind, string target);
    event PreferenceRemoved(address indexed subscriber, uint256 indexed index);
    event PreferenceActiveSet(address indexed subscriber, uint256 indexed index, bool active);

    function setUp() public {
        registry = new NotificationRegistry();
    }

    /// 1 — addPreference appends and emits with correct index
    function test_add_preference_appends_and_emits() public {
        vm.expectEmit(true, true, false, true, address(registry));
        emit PreferenceAdded(alice, 0, NotificationRegistry.Kind.Webhook, "https://example.com/hook");

        vm.prank(alice);
        uint256 idx = registry.addPreference(NotificationRegistry.Kind.Webhook, "https://example.com/hook");

        assertEq(idx, 0);
        assertEq(registry.preferenceCount(alice), 1);
    }

    /// 2 — multiple addPreferences grow the array and assign stable indices
    function test_add_multiple_preferences() public {
        vm.startPrank(alice);
        uint256 i0 = registry.addPreference(NotificationRegistry.Kind.Webhook, "https://a/1");
        uint256 i1 = registry.addPreference(NotificationRegistry.Kind.Email, "alice@example.com");
        uint256 i2 = registry.addPreference(NotificationRegistry.Kind.InApp, "alice-device-abc");
        vm.stopPrank();

        assertEq(i0, 0);
        assertEq(i1, 1);
        assertEq(i2, 2);
        assertEq(registry.preferenceCount(alice), 3);

        NotificationRegistry.Preference[] memory prefs = registry.getPreferences(alice);
        assertEq(prefs.length, 3);
        assertEq(uint8(prefs[0].kind), uint8(NotificationRegistry.Kind.Webhook));
        assertEq(uint8(prefs[1].kind), uint8(NotificationRegistry.Kind.Email));
        assertEq(uint8(prefs[2].kind), uint8(NotificationRegistry.Kind.InApp));
    }

    /// 3 — removePreference tombstones (does not shift) and emits
    function test_remove_preference_tombstones_and_emits() public {
        vm.startPrank(alice);
        registry.addPreference(NotificationRegistry.Kind.Webhook, "https://a/1");
        registry.addPreference(NotificationRegistry.Kind.Email, "alice@example.com");
        vm.stopPrank();

        vm.expectEmit(true, true, false, false, address(registry));
        emit PreferenceRemoved(alice, 0);
        vm.prank(alice);
        registry.removePreference(0);

        NotificationRegistry.Preference[] memory prefs = registry.getPreferences(alice);
        // Length preserved (not shifted)
        assertEq(prefs.length, 2);
        // Tombstoned slot
        assertFalse(prefs[0].active);
        assertEq(bytes(prefs[0].target).length, 0);
        // Other slot unchanged
        assertTrue(prefs[1].active);
        assertEq(prefs[1].target, "alice@example.com");
    }

    /// 4 — removePreference on an out-of-bounds index reverts
    function test_remove_out_of_bounds_reverts() public {
        vm.prank(alice);
        vm.expectRevert(NotificationRegistry.IndexOutOfBounds.selector);
        registry.removePreference(0);

        vm.prank(alice);
        registry.addPreference(NotificationRegistry.Kind.Webhook, "https://a/1");

        vm.prank(alice);
        vm.expectRevert(NotificationRegistry.IndexOutOfBounds.selector);
        registry.removePreference(5);
    }

    /// 5 — setActive toggles and emits
    function test_set_active_toggles_and_emits() public {
        vm.prank(alice);
        registry.addPreference(NotificationRegistry.Kind.Webhook, "https://a/1");

        vm.expectEmit(true, true, false, true, address(registry));
        emit PreferenceActiveSet(alice, 0, false);
        vm.prank(alice);
        registry.setActive(0, false);

        NotificationRegistry.Preference[] memory prefs = registry.getPreferences(alice);
        assertFalse(prefs[0].active);
        // Target preserved — unlike removePreference
        assertEq(prefs[0].target, "https://a/1");

        vm.prank(alice);
        registry.setActive(0, true);
        assertTrue(registry.getPreferences(alice)[0].active);
    }

    /// 6 — addPreference with empty target reverts
    function test_empty_target_reverts() public {
        vm.prank(alice);
        vm.expectRevert(NotificationRegistry.EmptyTarget.selector);
        registry.addPreference(NotificationRegistry.Kind.Webhook, "");
    }

    /// 7 — hasActivePreference accurately reflects state
    function test_has_active_preference_returns_correct() public {
        assertFalse(registry.hasActivePreference(alice));

        vm.prank(alice);
        registry.addPreference(NotificationRegistry.Kind.Webhook, "https://a/1");
        assertTrue(registry.hasActivePreference(alice));

        vm.prank(alice);
        registry.setActive(0, false);
        assertFalse(registry.hasActivePreference(alice));

        // Bob is independent
        assertFalse(registry.hasActivePreference(bob));
        vm.prank(bob);
        registry.addPreference(NotificationRegistry.Kind.Email, "bob@example.com");
        assertTrue(registry.hasActivePreference(bob));
    }
}
