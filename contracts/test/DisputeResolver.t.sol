// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";
import { MockERC20 } from "../src/MockERC20.sol";
import { WorkerRegistry } from "../src/WorkerRegistry.sol";
import { BountyMarket } from "../src/BountyMarket.sol";
import { DisputeResolver } from "../src/DisputeResolver.sol";

contract DisputeResolverTest is Test {
    MockERC20 internal token;
    WorkerRegistry internal workers;
    BountyMarket internal market;
    DisputeResolver internal resolver;

    address internal poster = address(0xBEEF);
    address internal worker = address(0xC0FFEE);
    address internal mediator = address(0x3E1A);
    address internal stranger = address(0xDEAD);

    event DisputeOpened(uint256 indexed jobId, address indexed opener);
    event EvidenceSubmitted(uint256 indexed jobId, address indexed worker, bytes32 pointer);
    event DisputeFinalized(uint256 indexed jobId, bool outcome, address indexed finalizer);
    event PassthroughResolved(uint256 indexed jobId, bool accepted);

    function setUp() public {
        token = new MockERC20("DAEJI", "DAEJI", 18);
        workers = new WorkerRegistry(address(token));
        market = new BountyMarket(address(token), address(workers));
        resolver = new DisputeResolver(address(market), mediator);

        workers.setAuthorized(address(market), true);
        // Transfer resolver authority from deployer to DisputeResolver.
        market.setResolver(address(resolver));

        token.mint(poster, 1_000_000 ether);
        token.mint(worker, 10_000 ether);

        vm.prank(poster);
        token.approve(address(market), type(uint256).max);
        vm.prank(worker);
        token.approve(address(workers), type(uint256).max);

        vm.prank(worker);
        workers.register(1_000 ether);
    }

    function _postAssignSubmit() internal returns (uint256 id) {
        vm.prank(poster);
        id = market.postJob(
            keccak256("spec"),
            500 ether,
            uint64(block.timestamp + 3600),
            uint8(WorkerRegistry.Tier.Standard)
        );
        market.assign(id, worker);
        vm.prank(worker);
        market.submit(id, keccak256("result"));
    }

    /// 1 — passthrough(accept) forwards to market and terminates the job
    function test_passthrough_accept_forwards_to_market() public {
        uint256 id = _postAssignSubmit();
        uint256 workerBalBefore = token.balanceOf(worker);

        vm.expectEmit(true, false, false, true, address(resolver));
        emit PassthroughResolved(id, true);
        vm.prank(mediator);
        resolver.passthrough(id, true);

        assertEq(uint8(market.stateOf(id)), uint8(BountyMarket.State.Terminal));
        assertEq(token.balanceOf(worker), workerBalBefore + 500 ether);
    }

    /// 2 — passthrough(reject) refunds poster + slashes worker
    function test_passthrough_reject_forwards_to_market() public {
        uint256 id = _postAssignSubmit();
        uint256 posterBalBefore = token.balanceOf(poster);
        uint256 workerBondBefore = workers.getWorker(worker).bond;

        vm.prank(mediator);
        resolver.passthrough(id, false);

        assertEq(uint8(market.stateOf(id)), uint8(BountyMarket.State.Terminal));
        assertEq(token.balanceOf(poster), posterBalBefore + 500 ether);
        // 5% slash (SLASH_QUALITY_REJECT path = 500 bps)
        assertEq(
            workers.getWorker(worker).bond,
            workerBondBefore - (workerBondBefore * 500) / 10_000
        );
    }

    /// 3 — non-mediator cannot passthrough
    function test_passthrough_only_mediator() public {
        uint256 id = _postAssignSubmit();
        vm.prank(stranger);
        vm.expectRevert(DisputeResolver.NotMediator.selector);
        resolver.passthrough(id, true);
    }

    /// 4 — openDispute transitions state to Pending
    function test_openDispute_transitions_to_pending() public {
        uint256 id = _postAssignSubmit();

        vm.expectEmit(true, true, false, false, address(resolver));
        emit DisputeOpened(id, mediator);
        vm.prank(mediator);
        resolver.openDispute(id);

        DisputeResolver.Dispute memory d = resolver.getDispute(id);
        assertEq(uint8(d.state), uint8(DisputeResolver.DisputeState.Pending));
        assertEq(d.openedAt, uint64(block.timestamp));
    }

    /// 5 — only assigned worker can submitEvidence
    function test_submitEvidence_only_by_assigned_worker() public {
        uint256 id = _postAssignSubmit();
        vm.prank(mediator);
        resolver.openDispute(id);

        vm.prank(stranger);
        vm.expectRevert(DisputeResolver.NotWorker.selector);
        resolver.submitEvidence(id, bytes32("evidence"));
    }

    /// 6 — submitEvidence transitions Pending → Responded and records pointer
    function test_submitEvidence_transitions_to_responded() public {
        uint256 id = _postAssignSubmit();
        vm.prank(mediator);
        resolver.openDispute(id);

        bytes32 pointer = keccak256("ipfs://evidence-bundle");
        vm.expectEmit(true, true, false, true, address(resolver));
        emit EvidenceSubmitted(id, worker, pointer);
        vm.prank(worker);
        resolver.submitEvidence(id, pointer);

        DisputeResolver.Dispute memory d = resolver.getDispute(id);
        assertEq(uint8(d.state), uint8(DisputeResolver.DisputeState.Responded));
        assertEq(d.evidencePointer, pointer);
    }

    /// 7 — finalizeDispute calls market.resolve and marks Adjudicated
    function test_finalizeDispute_calls_market_resolve_and_marks_adjudicated() public {
        uint256 id = _postAssignSubmit();
        vm.prank(mediator);
        resolver.openDispute(id);
        vm.prank(worker);
        resolver.submitEvidence(id, keccak256("ok"));

        uint256 workerBalBefore = token.balanceOf(worker);
        vm.prank(mediator);
        resolver.finalizeDispute(id, true);

        assertEq(uint8(market.stateOf(id)), uint8(BountyMarket.State.Terminal));
        assertEq(token.balanceOf(worker), workerBalBefore + 500 ether);
        assertEq(
            uint8(resolver.getDispute(id).state),
            uint8(DisputeResolver.DisputeState.Adjudicated)
        );
    }

    /// 8 — finalizeExpired before window reverts
    function test_finalizeExpired_before_window_reverts() public {
        uint256 id = _postAssignSubmit();
        vm.prank(mediator);
        resolver.openDispute(id);

        // Not yet past the window
        vm.expectRevert(DisputeResolver.WindowNotExpired.selector);
        resolver.finalizeExpired(id);
    }

    /// 9 — finalizeExpired with no response → default reject
    function test_finalizeExpired_no_response_defaults_reject() public {
        uint256 id = _postAssignSubmit();
        vm.prank(mediator);
        resolver.openDispute(id);

        vm.warp(block.timestamp + 48 hours + 1);
        uint256 posterBalBefore = token.balanceOf(poster);
        resolver.finalizeExpired(id); // permissionless

        assertEq(uint8(market.stateOf(id)), uint8(BountyMarket.State.Terminal));
        assertEq(token.balanceOf(poster), posterBalBefore + 500 ether, "refund to poster");
        assertFalse(resolver.getDispute(id).finalOutcome, "outcome == reject");
    }

    /// 10 — finalizeExpired with worker response → default accept
    function test_finalizeExpired_with_response_defaults_accept() public {
        uint256 id = _postAssignSubmit();
        vm.prank(mediator);
        resolver.openDispute(id);
        vm.prank(worker);
        resolver.submitEvidence(id, keccak256("response"));

        vm.warp(block.timestamp + 48 hours + 1);
        uint256 workerBalBefore = token.balanceOf(worker);
        resolver.finalizeExpired(id);

        assertEq(uint8(market.stateOf(id)), uint8(BountyMarket.State.Terminal));
        assertEq(token.balanceOf(worker), workerBalBefore + 500 ether, "payout to worker");
        assertTrue(resolver.getDispute(id).finalOutcome, "outcome == accept");
    }
}
