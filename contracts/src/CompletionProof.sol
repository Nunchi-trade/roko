// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { ConsortiumValidator } from "./ConsortiumValidator.sol";

/// @title CompletionProof — 2-of-3 validator attestation for BountyMarket resultHashes.
/// @notice Standalone read-only artifact. Committee members (pulled from
///         `ConsortiumValidator.getMembers(jobId)`) attest to a (jobId, resultHash)
///         pair. Once `QUORUM` members attest, the proof is marked complete and
///         `ProofComplete` is emitted. Additional attestations (3rd of 3) are
///         idempotent — they record the attestation but do not re-emit.
/// @dev    No signature verification: attestation is by `msg.sender` identity.
///         Signature-based attestation is a post-demo upgrade.
contract CompletionProof {
    struct Proof {
        bytes32 resultHash;
        uint8 attestCount;
        bool complete;
    }

    ConsortiumValidator public immutable consortium;
    uint8 public constant QUORUM = 2;

    mapping(uint256 => Proof) private _proofs;
    mapping(uint256 => mapping(address => bool)) private _attested;

    event AttestationSubmitted(uint256 indexed jobId, address indexed validator, bytes32 resultHash);
    event ProofComplete(uint256 indexed jobId, bytes32 resultHash);

    error NotCommitteeMember();
    error AlreadyAttested();
    error ResultHashMismatch();
    error ZeroAddress();

    constructor(address consortium_) {
        if (consortium_ == address(0)) revert ZeroAddress();
        consortium = ConsortiumValidator(consortium_);
    }

    /// @notice Committee member attests that `resultHash` is the correct result
    ///         for `jobId`. First attestation locks the hash; subsequent attesters
    ///         must submit a matching hash. On the QUORUM-th attestation, emits
    ///         `ProofComplete`. Additional attestations are recorded but do not
    ///         re-emit `ProofComplete`.
    function attest(uint256 jobId, bytes32 resultHash) external {
        if (!_isMember(jobId, msg.sender)) revert NotCommitteeMember();
        if (_attested[jobId][msg.sender]) revert AlreadyAttested();

        Proof storage p = _proofs[jobId];
        if (p.attestCount == 0) {
            p.resultHash = resultHash;
        } else if (p.resultHash != resultHash) {
            revert ResultHashMismatch();
        }

        _attested[jobId][msg.sender] = true;
        p.attestCount += 1;

        emit AttestationSubmitted(jobId, msg.sender, resultHash);

        if (!p.complete && p.attestCount >= QUORUM) {
            p.complete = true;
            emit ProofComplete(jobId, resultHash);
        }
    }

    function isProven(uint256 jobId) external view returns (bool) {
        return _proofs[jobId].complete;
    }

    function getProof(uint256 jobId)
        external
        view
        returns (bytes32 resultHash, uint8 attestCount, bool complete)
    {
        Proof storage p = _proofs[jobId];
        return (p.resultHash, p.attestCount, p.complete);
    }

    function hasAttested(uint256 jobId, address validator) external view returns (bool) {
        return _attested[jobId][validator];
    }

    function _isMember(uint256 jobId, address who) internal view returns (bool) {
        address[3] memory members = consortium.getMembers(jobId);
        return who == members[0] || who == members[1] || who == members[2];
    }
}
