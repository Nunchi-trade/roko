// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";
import { MockERC20 } from "../src/MockERC20.sol";
import { WorkerRegistry } from "../src/WorkerRegistry.sol";
import { BountyMarket } from "../src/BountyMarket.sol";
import { ConsortiumValidator } from "../src/ConsortiumValidator.sol";
import { CompletionProof } from "../src/CompletionProof.sol";

contract CompletionProofTest is Test {
    MockERC20 internal token;
    WorkerRegistry internal workers;
    BountyMarket internal market;
    ConsortiumValidator internal consortium;
    CompletionProof internal proof;

    address internal poster = address(0xBEEF);
    address internal jobWorker = address(0x5061E);   // Standard tier, executes the job
    address internal v0 = address(0x401);            // Trusted tier, committee
    address internal v1 = address(0x402);            // Trusted tier, committee
    address internal v2 = address(0x403);            // Trusted tier, committee
    address internal stranger = address(0xDEAD);

    bytes32 internal constant RESULT_HASH = keccak256("result");

    event AttestationSubmitted(uint256 indexed jobId, address indexed validator, bytes32 resultHash);
    event ProofComplete(uint256 indexed jobId, bytes32 resultHash);

    function setUp() public {
        token = new MockERC20("DAEJI", "DAEJI", 18);
        workers = new WorkerRegistry(address(token));
        market = new BountyMarket(address(token), address(workers));
        consortium = new ConsortiumValidator(address(workers), address(market));
        proof = new CompletionProof(address(consortium));

        workers.setAuthorized(address(market), true);
        workers.setAuthorized(address(consortium), true);
        workers.setAuthorized(address(this), true); // test contract boosts reputations directly

        token.mint(poster, 1_000_000 ether);

        vm.prank(poster);
        token.approve(address(market), type(uint256).max);

        _registerAndBond(jobWorker);      // starts at 0.5, Standard tier (no boost)
        _registerAndBond(v0);
        _registerAndBond(v1);
        _registerAndBond(v2);

        // Boost v0, v1, v2 to Trusted tier (>= 0.55). One updateReputation(true)
        // from 0.5 gives R_new = 0.2*1.0 + 0.8*0.5 = 0.6 → Trusted.
        workers.updateReputation(v0, true);
        workers.updateReputation(v1, true);
        workers.updateReputation(v2, true);
    }

    function _registerAndBond(address w) internal {
        token.mint(w, 10_000 ether);
        vm.prank(w);
        token.approve(address(workers), type(uint256).max);
        vm.prank(w);
        workers.register(1_000 ether);
    }

    /// @dev Post a job, assign `jobWorker`, submit RESULT_HASH, assemble a
    ///      3-member committee (drawn from v0/v1/v2, the only Trusted workers).
    function _stageAndAssembleCommittee() internal returns (uint256 id) {
        vm.prank(poster);
        id = market.postJob(
            keccak256("spec"),
            500 ether,
            uint64(block.timestamp + 3600),
            uint8(WorkerRegistry.Tier.Standard)
        );
        market.assign(id, jobWorker);
        vm.prank(jobWorker);
        market.submit(id, RESULT_HASH);

        // Advance block so `blockhash(block.number - 1)` is nonzero for the seed.
        vm.roll(block.number + 1);
        consortium.assembleCommittee(id);
    }

    /// 1 — first attestation stores the result hash
    function test_first_attestation_stores_result_hash() public {
        uint256 id = _stageAndAssembleCommittee();

        address[3] memory members = consortium.getMembers(id);
        vm.prank(members[0]);
        proof.attest(id, RESULT_HASH);

        (bytes32 storedHash, uint8 count, bool complete) = proof.getProof(id);
        assertEq(storedHash, RESULT_HASH);
        assertEq(count, 1);
        assertFalse(complete);
        assertTrue(proof.hasAttested(id, members[0]));
    }

    /// 2 — second attestation reaches quorum and emits ProofComplete
    function test_second_attestation_marks_proven() public {
        uint256 id = _stageAndAssembleCommittee();

        address[3] memory members = consortium.getMembers(id);
        vm.prank(members[0]);
        proof.attest(id, RESULT_HASH);

        vm.expectEmit(true, false, false, true, address(proof));
        emit ProofComplete(id, RESULT_HASH);
        vm.prank(members[1]);
        proof.attest(id, RESULT_HASH);

        assertTrue(proof.isProven(id));
        (, uint8 count, bool complete) = proof.getProof(id);
        assertEq(count, 2);
        assertTrue(complete);
    }

    /// 3 — third attestation is idempotent on the complete flag
    function test_third_attestation_idempotent() public {
        uint256 id = _stageAndAssembleCommittee();
        address[3] memory members = consortium.getMembers(id);

        vm.prank(members[0]);
        proof.attest(id, RESULT_HASH);
        vm.prank(members[1]);
        proof.attest(id, RESULT_HASH);
        // ProofComplete already emitted. Third attest increments count but does
        // NOT emit ProofComplete a second time. We don't check emit absence here
        // (forge doesn't assert-no-emit cleanly); we just verify state is sane.
        vm.prank(members[2]);
        proof.attest(id, RESULT_HASH);

        (, uint8 count, bool complete) = proof.getProof(id);
        assertEq(count, 3);
        assertTrue(complete);
    }

    /// 4 — non-committee-member cannot attest
    function test_attest_reverts_for_non_committee_member() public {
        uint256 id = _stageAndAssembleCommittee();

        vm.prank(stranger);
        vm.expectRevert(CompletionProof.NotCommitteeMember.selector);
        proof.attest(id, RESULT_HASH);
    }

    /// 5 — second attester must submit the same hash
    function test_attest_reverts_on_hash_mismatch() public {
        uint256 id = _stageAndAssembleCommittee();
        address[3] memory members = consortium.getMembers(id);

        vm.prank(members[0]);
        proof.attest(id, RESULT_HASH);

        vm.prank(members[1]);
        vm.expectRevert(CompletionProof.ResultHashMismatch.selector);
        proof.attest(id, keccak256("wrong"));
    }

    /// 6 — same validator cannot attest twice
    function test_attest_reverts_on_duplicate_attestation() public {
        uint256 id = _stageAndAssembleCommittee();
        address[3] memory members = consortium.getMembers(id);

        vm.prank(members[0]);
        proof.attest(id, RESULT_HASH);

        vm.prank(members[0]);
        vm.expectRevert(CompletionProof.AlreadyAttested.selector);
        proof.attest(id, RESULT_HASH);
    }

    /// 7 — isProven flips false → true at quorum
    function test_isProven_returns_correct_state() public {
        uint256 id = _stageAndAssembleCommittee();
        assertFalse(proof.isProven(id));

        address[3] memory members = consortium.getMembers(id);
        vm.prank(members[0]);
        proof.attest(id, RESULT_HASH);
        assertFalse(proof.isProven(id));

        vm.prank(members[1]);
        proof.attest(id, RESULT_HASH);
        assertTrue(proof.isProven(id));
    }
}
