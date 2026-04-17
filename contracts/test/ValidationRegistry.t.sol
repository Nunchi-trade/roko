// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";

import { IdentityRegistry } from "../src/IdentityRegistry.sol";
import { ValidationRegistry } from "../src/ValidationRegistry.sol";

contract ValidationRegistryTest is Test {
    IdentityRegistry internal identity;
    ValidationRegistry internal validation;

    address internal alice = address(0xA11CE);
    address internal bob = address(0xB0B);
    address internal carol = address(0xCA201);
    address internal dave = address(0xDA7E);
    address internal market = address(0xCAFE);

    function setUp() public {
        identity = new IdentityRegistry(address(this), address(0));
        validation = new ValidationRegistry(address(identity), address(this));

        vm.prank(alice);
        identity.registerPassport(alice, 0, keccak256("alice"), bytes32(0), 0);
        vm.prank(bob);
        identity.registerPassport(bob, 0, keccak256("bob"), bytes32(0), 0);
        vm.prank(carol);
        identity.registerPassport(carol, 0, keccak256("carol"), bytes32(0), 0);
        vm.prank(dave);
        identity.registerPassport(dave, 0, keccak256("dave"), bytes32(0), 0);
    }

    function test_submitWorkProof_andQueryProofs() public {
        uint256 passportId = identity.ownerToPassportId(alice);
        bytes32 jobHash = keccak256("job-hash");

        uint64 beforeBlock = uint64(block.number);
        uint8[] memory gateResults = new uint8[](4);
        gateResults[0] = 1;
        gateResults[1] = 1;
        gateResults[2] = 0;
        gateResults[3] = 1;

        vm.prank(alice);
        validation.submitWorkProof(passportId, jobHash, keccak256("root"), gateResults, hex"1234");

        ValidationRegistry.WorkProof memory proof = validation.verifyWork(jobHash);
        assertEq(proof.passportId, passportId);
        assertEq(proof.jobHash, jobHash);
        assertEq(proof.gateResults.length, 4);
        assertEq(keccak256(proof.clearingCert), keccak256(hex"1234"));

        ValidationRegistry.WorkProof[] memory proofs =
            validation.getWorkProofs(passportId, beforeBlock, uint64(block.number));
        assertEq(proofs.length, 1);
        assertEq(proofs[0].jobHash, jobHash);

        (uint256 passRate, uint64 totalJobs) = validation.getGatePassRate(passportId, "security");
        assertEq(totalJobs, 1);
        assertEq(passRate, 75e16);
    }

    function test_authorizedSubmitter_canPublishForPassportOwner() public {
        uint256 passportId = identity.ownerToPassportId(alice);
        validation.setAuthorizedSubmitter(market, true);

        uint8[] memory gateResults = new uint8[](2);
        gateResults[0] = 1;
        gateResults[1] = 1;

        vm.prank(market);
        validation.submitWorkProof(passportId, keccak256("market-job"), keccak256("root"), gateResults, "");

        ValidationRegistry.WorkProof memory proof = validation.verifyWork(keccak256("market-job"));
        assertEq(proof.passportId, passportId);
    }

    function test_requestValidation_collectsAttestations_andResolvesAfterThree() public {
        bytes32 workHash = keccak256("work");
        bytes32 taskId = keccak256("task");

        vm.prank(alice);
        bytes32 requestId =
            validation.requestValidation(workHash, taskId, ValidationRegistry.ValidatorType.ReputationBased);

        vm.prank(bob);
        validation.submitAttestation(requestId, true, keccak256("evidence-1"));
        vm.prank(carol);
        validation.submitAttestation(requestId, true, keccak256("evidence-2"));
        vm.prank(dave);
        validation.submitAttestation(requestId, false, keccak256("evidence-3"));

        ValidationRegistry.ValidationRequest memory request = validation.getRequest(requestId);
        ValidationRegistry.ValidationAttestation[] memory attestations = validation.getAttestations(requestId);

        assertTrue(request.resolved);
        assertEq(attestations.length, 3);
        assertEq(attestations[0].validatorPassportId, identity.ownerToPassportId(bob));
    }
}
