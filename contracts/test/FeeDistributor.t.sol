// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import { Test } from "forge-std/Test.sol";
import { MockERC20 } from "../src/MockERC20.sol";
import { FeeDistributor } from "../src/FeeDistributor.sol";

contract FeeDistributorTest is Test {
    MockERC20 internal token;
    FeeDistributor internal distributor;

    address internal treasury = address(0x7000);
    address internal payer = address(0xBEEF);
    address internal winner = address(0xC0FFEE);
    address internal validatorA = address(0x1111);
    address internal validatorB = address(0x2222);
    address internal dataA = address(0x3333);
    address internal dataB = address(0x4444);

    event FeesDistributed(
        uint256 indexed jobId,
        uint256 amount,
        address indexed winner,
        uint256 validatorShare,
        uint256 dataShare,
        uint256 agentShare,
        uint256 treasuryShare
    );
    event EarningsCredited(address indexed participant, uint256 amount);

    function setUp() public {
        token = new MockERC20("DAEJI", "DAEJI", 18);
        distributor = new FeeDistributor(address(token), treasury);

        token.mint(payer, 1_000_000 ether);
        vm.prank(payer);
        token.approve(address(distributor), type(uint256).max);
    }

    function test_basic_split() public {
        address[] memory validators = new address[](2);
        validators[0] = validatorA;
        validators[1] = validatorB;
        address[] memory dataProviders = new address[](2);
        dataProviders[0] = dataA;
        dataProviders[1] = dataB;

        vm.prank(payer);
        distributor.distribute(1, 1_000 ether, winner, validators, dataProviders);

        assertEq(token.balanceOf(validatorA), 200 ether);
        assertEq(token.balanceOf(validatorB), 200 ether);
        assertEq(token.balanceOf(dataA), 150 ether);
        assertEq(token.balanceOf(dataB), 150 ether);
        assertEq(token.balanceOf(winner), 200 ether);
        assertEq(token.balanceOf(treasury), 100 ether);
    }

    function test_empty_validators_rolls_share_to_treasury() public {
        address[] memory validators = new address[](0);
        address[] memory dataProviders = new address[](1);
        dataProviders[0] = dataA;

        vm.prank(payer);
        distributor.distribute(2, 1_000 ether, winner, validators, dataProviders);

        assertEq(token.balanceOf(dataA), 300 ether);
        assertEq(token.balanceOf(winner), 200 ether);
        assertEq(token.balanceOf(treasury), 500 ether);
    }

    function test_empty_data_providers_rolls_share_to_treasury() public {
        address[] memory validators = new address[](2);
        validators[0] = validatorA;
        validators[1] = validatorB;
        address[] memory dataProviders = new address[](0);

        vm.prank(payer);
        distributor.distribute(3, 1_000 ether, winner, validators, dataProviders);

        assertEq(token.balanceOf(validatorA), 200 ether);
        assertEq(token.balanceOf(validatorB), 200 ether);
        assertEq(token.balanceOf(winner), 200 ether);
        assertEq(token.balanceOf(treasury), 400 ether);
    }

    function test_single_validator_gets_entire_validator_share() public {
        address[] memory validators = new address[](1);
        validators[0] = validatorA;
        address[] memory dataProviders = new address[](2);
        dataProviders[0] = dataA;
        dataProviders[1] = dataB;

        vm.prank(payer);
        distributor.distribute(4, 1_000 ether, winner, validators, dataProviders);

        assertEq(token.balanceOf(validatorA), 400 ether);
    }

    function test_cumulative_earnings_accumulate() public {
        address[] memory validators = new address[](1);
        validators[0] = validatorA;
        address[] memory dataProviders = new address[](1);
        dataProviders[0] = dataA;

        vm.startPrank(payer);
        distributor.distribute(5, 1_000 ether, winner, validators, dataProviders);
        distributor.distribute(6, 500 ether, winner, validators, dataProviders);
        vm.stopPrank();

        assertEq(distributor.cumulativeEarnings(validatorA), 600 ether);
        assertEq(distributor.cumulativeEarnings(dataA), 450 ether);
        assertEq(distributor.cumulativeEarnings(winner), 300 ether);
        assertEq(distributor.cumulativeEarnings(treasury), 150 ether);
    }

    function test_events_emitted() public {
        address[] memory validators = new address[](1);
        validators[0] = validatorA;
        address[] memory dataProviders = new address[](1);
        dataProviders[0] = dataA;

        vm.expectEmit(true, true, false, true);
        emit EarningsCredited(validatorA, 400 ether);
        vm.expectEmit(true, true, false, true);
        emit EarningsCredited(dataA, 300 ether);
        vm.expectEmit(true, true, false, true);
        emit EarningsCredited(winner, 200 ether);
        vm.expectEmit(true, true, false, true);
        emit EarningsCredited(treasury, 100 ether);
        vm.expectEmit(true, true, false, true);
        emit FeesDistributed(7, 1_000 ether, winner, 400 ether, 300 ether, 200 ether, 100 ether);

        vm.prank(payer);
        distributor.distribute(7, 1_000 ether, winner, validators, dataProviders);
    }

    function test_rounding_no_tokens_lost() public {
        address[] memory validators = new address[](3);
        validators[0] = validatorA;
        validators[1] = validatorB;
        validators[2] = address(0x5555);
        address[] memory dataProviders = new address[](2);
        dataProviders[0] = dataA;
        dataProviders[1] = dataB;

        uint256 amount = 1_001;
        uint256 payerBefore = token.balanceOf(payer);

        vm.prank(payer);
        distributor.distribute(8, amount, winner, validators, dataProviders);

        uint256 totalPaid =
            token.balanceOf(validatorA)
            + token.balanceOf(validatorB)
            + token.balanceOf(address(0x5555))
            + token.balanceOf(dataA)
            + token.balanceOf(dataB)
            + token.balanceOf(winner)
            + token.balanceOf(treasury);

        assertEq(totalPaid, amount);
        assertEq(token.balanceOf(payer), payerBefore - amount);
    }
}
