// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";
import { MockERC20 } from "../src/MockERC20.sol";
import { FeeDistributor } from "../src/FeeDistributor.sol";

contract FeeDistributorTest is Test {
    MockERC20 internal token;
    FeeDistributor internal dist;

    address internal treasury = address(0x7EA5);
    address internal payer = address(0xBEEF);
    address internal agent = address(0xA6E7);
    address internal provider = address(0xD474);
    address internal val0 = address(0x1111);
    address internal val1 = address(0x2222);
    address internal val2 = address(0x3333);

    event FeesDistributed(
        uint256 indexed jobId,
        address indexed agent,
        address indexed provider,
        address[] validators,
        uint256 totalFee,
        uint256 agentShare,
        uint256 providerShare,
        uint256 validatorTotalShare,
        uint256 treasuryShare
    );

    function setUp() public {
        token = new MockERC20("DAEJI", "DAEJI", 18);
        dist = new FeeDistributor(address(token), treasury);
        token.mint(payer, 1_000_000 ether);
        vm.prank(payer);
        token.approve(address(dist), type(uint256).max);
    }

    function _singleValidator() internal view returns (address[] memory v) {
        v = new address[](1);
        v[0] = val0;
    }

    function _threeValidators() internal view returns (address[] memory v) {
        v = new address[](3);
        v[0] = val0;
        v[1] = val1;
        v[2] = val2;
    }

    /// 1 — Basic 40/30/20/10 split with a single validator receiving the full 40%.
    function test_distribute_splits_40_30_20_10() public {
        address[] memory v = _singleValidator();
        vm.prank(payer);
        dist.distribute(1, agent, provider, v, 100 ether);

        assertEq(dist.pendingOf(agent), 20 ether, "agent 20%");
        assertEq(dist.pendingOf(provider), 30 ether, "provider 30%");
        assertEq(dist.pendingOf(val0), 40 ether, "single validator 40%");
        assertEq(dist.pendingOf(treasury), 10 ether, "treasury 10%");
        assertEq(dist.totalDistributed(), 100 ether);
    }

    /// 2 — Validator slice splits evenly across 3 validators.
    function test_distribute_equal_split_across_3_validators() public {
        address[] memory v = _threeValidators();
        vm.prank(payer);
        // 30 ether → 40% = 12 ether validator total → 4 ether each.
        dist.distribute(2, agent, provider, v, 30 ether);

        assertEq(dist.pendingOf(val0), 4 ether);
        assertEq(dist.pendingOf(val1), 4 ether);
        assertEq(dist.pendingOf(val2), 4 ether);
        assertEq(dist.pendingOf(agent), 6 ether);
        assertEq(dist.pendingOf(provider), 9 ether);
        assertEq(dist.pendingOf(treasury), 3 ether);
    }

    /// 3 — Rounding: totalFee indivisible by BPS_DENOM; dust must land in treasury
    ///     so sum(shares) == totalFee exactly.
    function test_distribute_rounding_credits_treasury() public {
        address[] memory v = _singleValidator();
        uint256 fee = 10_003; // 10_003 % 10_000 != 0 → rounding dust exists
        vm.prank(payer);
        dist.distribute(3, agent, provider, v, fee);

        uint256 agentShare = (fee * 2_000) / 10_000;       // 2_000
        uint256 providerShare = (fee * 3_000) / 10_000;    // 3_000
        uint256 validatorShare = (fee * 4_000) / 10_000;   // 4_001
        uint256 treasuryShare = fee - agentShare - providerShare - validatorShare; // 1_002 (absorbs dust)

        assertEq(dist.pendingOf(agent), agentShare);
        assertEq(dist.pendingOf(provider), providerShare);
        assertEq(dist.pendingOf(val0), validatorShare);
        assertEq(dist.pendingOf(treasury), treasuryShare);
        // Full conservation: no wei lost.
        assertEq(
            dist.pendingOf(agent)
                + dist.pendingOf(provider)
                + dist.pendingOf(val0)
                + dist.pendingOf(treasury),
            fee
        );
    }

    /// 4 — Uneven validator split: validatorTotal indivisible by validator count.
    ///     Remainder must go to validators[0] for deterministic, gas-free handling.
    function test_distribute_validator_remainder_to_first() public {
        address[] memory v = _threeValidators();
        // fee = 28 wei → validatorTotal = 28*4000/10000 = 11 → per = 3, remainder = 2.
        uint256 fee = 28;
        vm.prank(payer);
        dist.distribute(4, agent, provider, v, fee);

        // Validators: 3 + remainder 2 → val0 = 5, val1 = 3, val2 = 3
        assertEq(dist.pendingOf(val0), 5, "val0 gets remainder");
        assertEq(dist.pendingOf(val1), 3);
        assertEq(dist.pendingOf(val2), 3);
    }

    /// 5 — Empty validator array must revert.
    function test_distribute_reverts_on_empty_validators() public {
        address[] memory empty = new address[](0);
        vm.prank(payer);
        vm.expectRevert(FeeDistributor.EmptyValidators.selector);
        dist.distribute(5, agent, provider, empty, 100 ether);
    }

    /// 6 — Event emission carries correct split amounts.
    function test_distribute_emits_fees_distributed_event() public {
        address[] memory v = _singleValidator();
        vm.prank(payer);
        vm.expectEmit(true, true, true, true, address(dist));
        emit FeesDistributed(
            6,
            agent,
            provider,
            v,
            100 ether,
            20 ether, // agentShare
            30 ether, // providerShare
            40 ether, // validatorTotalShare
            10 ether  // treasuryShare
        );
        dist.distribute(6, agent, provider, v, 100 ether);
    }

    /// 7 — Claim transfers pending earnings and zeroes the credit; re-claim reverts.
    function test_claim_transfers_and_resets_earnings() public {
        address[] memory v = _singleValidator();
        vm.prank(payer);
        dist.distribute(7, agent, provider, v, 100 ether);

        uint256 before = token.balanceOf(agent);
        vm.prank(agent);
        uint256 claimed = dist.claim();
        assertEq(claimed, 20 ether);
        assertEq(token.balanceOf(agent), before + 20 ether);
        assertEq(dist.pendingOf(agent), 0);

        // Second claim without new earnings reverts.
        vm.prank(agent);
        vm.expectRevert(FeeDistributor.NothingToClaim.selector);
        dist.claim();
    }
}
