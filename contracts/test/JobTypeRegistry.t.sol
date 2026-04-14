// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";
import { JobTypeRegistry } from "../src/JobTypeRegistry.sol";

contract JobTypeRegistryTest is Test {
    JobTypeRegistry internal registry;

    address internal admin = address(0xA017);
    address internal stranger = address(0xDEAD);

    bytes32 internal constant ORACLE_UPDATE = keccak256("oracle-update");
    bytes32 internal constant FUNDING_WINDOW = keccak256("funding-window");

    event JobTypeRegistered(bytes32 indexed jobType, string description, uint8 minTier, uint256 minBounty);
    event JobTypeDeactivated(bytes32 indexed jobType);
    event JobTypeReactivated(bytes32 indexed jobType);

    function setUp() public {
        registry = new JobTypeRegistry(admin);
    }

    function _registerOracle() internal {
        vm.prank(admin);
        registry.register(
            ORACLE_UPDATE,
            "Push the latest oracle price for a market",
            uint8(2),              // minTier = Standard
            100 ether,
            1 hours,
            "ipfs://Qm.../oracle.json"
        );
    }

    /// 1 — register appends and emits
    function test_register_appends_and_emits() public {
        vm.expectEmit(true, false, false, true, address(registry));
        emit JobTypeRegistered(ORACLE_UPDATE, "Push the latest oracle price for a market", 2, 100 ether);
        _registerOracle();

        assertEq(registry.jobTypeCount(), 1);
        assertTrue(registry.exists(ORACLE_UPDATE));
        assertTrue(registry.isActive(ORACLE_UPDATE));

        JobTypeRegistry.JobTemplate memory t = registry.getTemplate(ORACLE_UPDATE);
        assertEq(t.description, "Push the latest oracle price for a market");
        assertEq(t.minTier, 2);
        assertEq(t.minBounty, 100 ether);
        assertEq(t.maxDeadlineOffset, 1 hours);
        assertTrue(t.active);
    }

    /// 2 — register reverts on duplicate jobType key
    function test_register_reverts_on_duplicate() public {
        _registerOracle();
        vm.prank(admin);
        vm.expectRevert(JobTypeRegistry.AlreadyRegistered.selector);
        registry.register(ORACLE_UPDATE, "duplicate", 2, 100 ether, 1 hours, "");
    }

    /// 3 — update preserves existence and active flag, mutates other fields
    function test_update_preserves_existence() public {
        _registerOracle();

        vm.prank(admin);
        registry.update(
            ORACLE_UPDATE,
            "Updated description",
            uint8(3),
            200 ether,
            2 hours,
            "ipfs://Qm.../v2.json"
        );

        JobTypeRegistry.JobTemplate memory t = registry.getTemplate(ORACLE_UPDATE);
        assertEq(t.description, "Updated description");
        assertEq(t.minTier, 3);
        assertEq(t.minBounty, 200 ether);
        assertEq(t.maxDeadlineOffset, 2 hours);
        assertTrue(t.active, "active flag unchanged by update");
    }

    /// 4 — deactivate toggles active false
    function test_deactivate_toggles_active() public {
        _registerOracle();
        assertTrue(registry.isActive(ORACLE_UPDATE));

        vm.expectEmit(true, false, false, false, address(registry));
        emit JobTypeDeactivated(ORACLE_UPDATE);
        vm.prank(admin);
        registry.deactivate(ORACLE_UPDATE);

        assertFalse(registry.isActive(ORACLE_UPDATE));
        assertTrue(registry.exists(ORACLE_UPDATE), "still exists after deactivate (tombstone)");
    }

    /// 5 — reactivate toggles active back true
    function test_reactivate_toggles_back() public {
        _registerOracle();
        vm.prank(admin);
        registry.deactivate(ORACLE_UPDATE);
        assertFalse(registry.isActive(ORACLE_UPDATE));

        vm.expectEmit(true, false, false, false, address(registry));
        emit JobTypeReactivated(ORACLE_UPDATE);
        vm.prank(admin);
        registry.reactivate(ORACLE_UPDATE);

        assertTrue(registry.isActive(ORACLE_UPDATE));
    }

    /// 6 — allJobTypes enumerates in insertion order
    function test_allJobTypes_enumerates() public {
        _registerOracle();
        vm.prank(admin);
        registry.register(FUNDING_WINDOW, "Settle one funding window for a market", 2, 50 ether, 30 minutes, "");

        bytes32[] memory all = registry.allJobTypes();
        assertEq(all.length, 2);
        assertEq(all[0], ORACLE_UPDATE);
        assertEq(all[1], FUNDING_WINDOW);
        assertEq(registry.jobTypeCount(), 2);
    }
}
