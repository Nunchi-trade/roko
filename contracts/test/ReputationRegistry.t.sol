// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";

import { IdentityRegistry } from "../src/IdentityRegistry.sol";
import { ReputationRegistry } from "../src/ReputationRegistry.sol";

contract ReputationRegistryTest is Test {
    IdentityRegistry internal identity;
    ReputationRegistry internal reputation;

    address internal alice = address(0xA11CE);
    address internal bob = address(0xB0B);
    address internal market = address(0xCAFE);

    function setUp() public {
        identity = new IdentityRegistry(address(this), address(0));
        reputation = new ReputationRegistry(address(identity), address(this));
        reputation.addFeedbackSource(market);

        vm.prank(alice);
        identity.registerPassport(alice, 0, keccak256("alice"), bytes32(0), 0);
        vm.prank(bob);
        identity.registerPassport(bob, 0, keccak256("bob"), bytes32(0), 0);
    }

    function test_authorizedSourceUpdatesDomainReputation() public {
        uint256 alicePassport = identity.ownerToPassportId(alice);

        vm.prank(market);
        reputation.submitFeedback(alicePassport, "solidity", int256(1e18), bytes32("job"), "clean run");

        (uint256 score, uint64 jobs,) = reputation.getReputation(alicePassport, "solidity");
        assertEq(jobs, 1);
        assertGt(score, reputation.SCALE() / 2);

        vm.expectRevert(ReputationRegistry.NotAuthorized.selector);
        vm.prank(bob);
        reputation.submitFeedback(alicePassport, "solidity", int256(1e18), bytes32("job"), "bad");
    }

    function test_peerAuthorizationFlow_updatesNamedDomain() public {
        uint256 alicePassport = identity.ownerToPassportId(alice);
        uint256 bobPassport = identity.ownerToPassportId(bob);

        vm.prank(market);
        reputation.authorizeFeedback(bobPassport, alicePassport, uint8(ReputationRegistry.ReputationDomain.KnowledgeVerification));

        vm.prank(bob);
        reputation.submitFeedback(alicePassport, uint8(ReputationRegistry.ReputationDomain.KnowledgeVerification), 800, bytes32("job-2"));

        (uint256 score, uint64 jobs,) = reputation.getReputation(alicePassport, "KnowledgeVerification");
        assertEq(jobs, 1);
        assertGt(score, reputation.SCALE() / 2);
    }

    function test_decayAndSlashHistory() public {
        uint256 alicePassport = identity.ownerToPassportId(alice);

        vm.prank(market);
        reputation.submitFeedback(alicePassport, "security", int256(1e18), bytes32("job-3"), "verified");

        (uint256 beforeDecay,,) = reputation.getReputation(alicePassport, "security");
        vm.warp(block.timestamp + reputation.DECAY_PERIOD());

        reputation.applyDecayTick(alicePassport);
        (uint256 afterDecay,,) = reputation.getReputation(alicePassport, "security");

        assertTrue(afterDecay < beforeDecay);
        assertApproxEqAbs(afterDecay, (beforeDecay + reputation.SCALE() / 2) / 2, 1);

        vm.prank(market);
        reputation.slash(alicePassport, 2, 250, "quality rejection");

        ReputationRegistry.SlashRecord[] memory history = reputation.getSlashHistory(alicePassport);
        assertEq(history.length, 1);
        assertEq(history[0].violationType, 2);
        assertEq(history[0].amount, 250);
        assertEq(history[0].reason, "quality rejection");
    }
}
