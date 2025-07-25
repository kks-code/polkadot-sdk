// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

///! Staking, and election related pallet configurations.
use super::*;
use cumulus_primitives_core::relay_chain::SessionIndex;
use frame_election_provider_support::{ElectionDataProvider, SequentialPhragmen};
use frame_support::traits::{ConstU128, EitherOf};
use pallet_election_provider_multi_block::{self as multi_block, SolutionAccuracyOf};
use pallet_staking_async::UseValidatorsMap;
use pallet_staking_async_rc_client as rc_client;
use polkadot_runtime_common::{prod_or_fast, BalanceToU256, U256ToBalance};
use sp_runtime::{
	transaction_validity::TransactionPriority, FixedPointNumber, FixedU128, SaturatedConversion,
};
use xcm::latest::prelude::*;

parameter_types! {
	/// Number of election pages that we operate upon. 32 * 6s block = 192s = 3.2min snapshots
	pub Pages: u32 = 32;

	/// Compatible with Polkadot, we allow up to 22_500 nominators to be considered for election
	pub MaxElectingVoters: u32 = 22_500;

	/// Maximum number of validators that we may want to elect. 1000 is the end target.
	pub const MaxValidatorSet: u32 = 1000;

	/// Number of nominators per page of the snapshot, and consequently number of backers in the solution.
	pub VoterSnapshotPerBlock: u32 = MaxElectingVoters::get() / Pages::get();

	/// Number of validators per page of the snapshot.
	pub TargetSnapshotPerBlock: u32 = MaxValidatorSet::get();

	// 10 mins for each pages
	pub storage SignedPhase: u32 = prod_or_fast!(
		10 * MINUTES,
		4 * MINUTES
	);
	pub storage UnsignedPhase: u32 = prod_or_fast!(
		10 * MINUTES,
		(1 * MINUTES)
	);

	/// validate up to 4 signed solution. Each solution.
	pub storage SignedValidationPhase: u32 = prod_or_fast!(Pages::get() * 4, Pages::get());

	/// In each page, we may observe up to all of the validators.
	pub MaxWinnersPerPage: u32 = MaxValidatorSet::get();

	/// In each page of the election, we allow up to all of the nominators of that page to be present.
	pub MaxBackersPerWinner: u32 = VoterSnapshotPerBlock::get();

	/// Total number of backers per winner across all pages.
	pub MaxBackersPerWinnerFinal: u32 = MaxElectingVoters::get();

	/// Size of the exposures. This should be small enough to make the reward payouts feasible.
	pub MaxExposurePageSize: u32 = 64;

	/// Each solution is considered "better" if it is 0.01% better.
	pub SolutionImprovementThreshold: Perbill = Perbill::from_rational(1u32, 10_000);
}

frame_election_provider_support::generate_solution_type!(
	#[compact]
	pub struct NposCompactSolution16::<
		// allows up to 4bn nominators
		VoterIndex = u32,
		// allows up to 64k validators
		TargetIndex = u16,
		Accuracy = sp_runtime::PerU16,
		MaxVoters = VoterSnapshotPerBlock,
	>(16)
);

ord_parameter_types! {
	// https://westend.subscan.io/account/5GBoBNFP9TA7nAk82i6SUZJimerbdhxaRgyC2PVcdYQMdb8e
	pub const WestendStakingMiner: AccountId = AccountId::from(hex_literal::hex!("b65991822483a6c3bd24b1dcf6afd3e270525da1f9c8c22a4373d1e1079e236a"));
}

impl multi_block::Config for Runtime {
	type Pages = Pages;
	type UnsignedPhase = UnsignedPhase;
	type SignedPhase = SignedPhase;
	type SignedValidationPhase = SignedValidationPhase;
	type VoterSnapshotPerBlock = VoterSnapshotPerBlock;
	type TargetSnapshotPerBlock = TargetSnapshotPerBlock;
	type AdminOrigin =
		EitherOfDiverse<EnsureRoot<AccountId>, EnsureSignedBy<WestendStakingMiner, AccountId>>;
	type DataProvider = Staking;
	type MinerConfig = Self;
	type Verifier = MultiBlockElectionVerifier;
	// we chill and do nothing in the fallback.
	type Fallback = multi_block::Continue<Self>;
	// Revert back to signed phase if nothing is submitted and queued, so we prolong the election.
	type AreWeDone = multi_block::RevertToSignedIfNotQueuedOf<Self>;
	type OnRoundRotation = multi_block::CleanRound<Self>;
	type WeightInfo = multi_block::weights::westend::MultiBlockWeightInfo<Self>;
}

impl multi_block::verifier::Config for Runtime {
	type MaxWinnersPerPage = MaxWinnersPerPage;
	type MaxBackersPerWinner = MaxBackersPerWinner;
	type MaxBackersPerWinnerFinal = MaxBackersPerWinnerFinal;
	type SolutionDataProvider = MultiBlockElectionSigned;
	type SolutionImprovementThreshold = SolutionImprovementThreshold;
	type WeightInfo = multi_block::weights::westend::MultiBlockVerifierWeightInfo<Self>;
}

parameter_types! {
	pub BailoutGraceRatio: Perbill = Perbill::from_percent(50);
	pub EjectGraceRatio: Perbill = Perbill::from_percent(50);
	pub DepositBase: Balance = 5 * UNITS;
	pub DepositPerPage: Balance = 1 * UNITS;
	pub RewardBase: Balance = 10 * UNITS;
	pub MaxSubmissions: u32 = 8;
}

impl multi_block::signed::Config for Runtime {
	type Currency = Balances;
	type BailoutGraceRatio = BailoutGraceRatio;
	type EjectGraceRatio = EjectGraceRatio;
	type DepositBase = DepositBase;
	type DepositPerPage = DepositPerPage;
	type InvulnerableDeposit = ();
	type RewardBase = RewardBase;
	type MaxSubmissions = MaxSubmissions;
	type EstimateCallFee = TransactionPayment;
	type WeightInfo = multi_block::weights::westend::MultiBlockSignedWeightInfo<Self>;
}

parameter_types! {
	/// Priority of the offchain miner transactions.
	pub MinerTxPriority: TransactionPriority = TransactionPriority::max_value() / 2;
	/// Try and run the OCW miner 4 times during the unsigned phase.
	pub OffchainRepeat: BlockNumber = UnsignedPhase::get() / 4;
	pub storage MinerPages: u32 = 2;
}

impl multi_block::unsigned::Config for Runtime {
	type MinerPages = MinerPages;
	type OffchainStorage = ConstBool<true>;
	type OffchainSolver = SequentialPhragmen<AccountId, SolutionAccuracyOf<Runtime>>;
	type MinerTxPriority = MinerTxPriority;
	type OffchainRepeat = OffchainRepeat;
	type WeightInfo = multi_block::weights::westend::MultiBlockUnsignedWeightInfo<Self>;
}

parameter_types! {
	/// Miner transaction can fill up to 75% of the block size.
	pub MinerMaxLength: u32 = Perbill::from_rational(75u32, 100) *
		*RuntimeBlockLength::get()
		.max
		.get(DispatchClass::Normal);
}

impl multi_block::unsigned::miner::MinerConfig for Runtime {
	type AccountId = AccountId;
	type Hash = Hash;
	type MaxBackersPerWinner = <Self as multi_block::verifier::Config>::MaxBackersPerWinner;
	type MaxBackersPerWinnerFinal =
		<Self as multi_block::verifier::Config>::MaxBackersPerWinnerFinal;
	type MaxWinnersPerPage = <Self as multi_block::verifier::Config>::MaxWinnersPerPage;
	type MaxVotesPerVoter =
		<<Self as multi_block::Config>::DataProvider as ElectionDataProvider>::MaxVotesPerVoter;
	type MaxLength = MinerMaxLength;
	type Solver = <Runtime as multi_block::unsigned::Config>::OffchainSolver;
	type Pages = Pages;
	type Solution = NposCompactSolution16;
	type VoterSnapshotPerBlock = <Runtime as multi_block::Config>::VoterSnapshotPerBlock;
	type TargetSnapshotPerBlock = <Runtime as multi_block::Config>::TargetSnapshotPerBlock;
}

parameter_types! {
	pub const BagThresholds: &'static [u64] = &bag_thresholds::THRESHOLDS;
}

type VoterBagsListInstance = pallet_bags_list::Instance1;
impl pallet_bags_list::Config<VoterBagsListInstance> for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type ScoreProvider = Staking;
	type WeightInfo = weights::pallet_bags_list::WeightInfo<Runtime>;
	type BagThresholds = BagThresholds;
	type Score = sp_npos_elections::VoteWeight;
}

pub struct EraPayout;
impl pallet_staking_async::EraPayout<Balance> for EraPayout {
	fn era_payout(
		_total_staked: Balance,
		_total_issuance: Balance,
		era_duration_millis: u64,
	) -> (Balance, Balance) {
		const MILLISECONDS_PER_YEAR: u64 = (1000 * 3600 * 24 * 36525) / 100;
		// A normal-sized era will have 1 / 365.25 here:
		let relative_era_len =
			FixedU128::from_rational(era_duration_millis.into(), MILLISECONDS_PER_YEAR.into());

		// Fixed total TI that we use as baseline for the issuance.
		let fixed_total_issuance: i128 = 5_216_342_402_773_185_773;
		let fixed_inflation_rate = FixedU128::from_rational(8, 100);
		let yearly_emission = fixed_inflation_rate.saturating_mul_int(fixed_total_issuance);

		let era_emission = relative_era_len.saturating_mul_int(yearly_emission);
		// 15% to treasury, as per Polkadot ref 1139.
		let to_treasury = FixedU128::from_rational(15, 100).saturating_mul_int(era_emission);
		let to_stakers = era_emission.saturating_sub(to_treasury);

		(to_stakers.saturated_into(), to_treasury.saturated_into())
	}
}

parameter_types! {
	// Six sessions in an era (6 hours).
	pub const SessionsPerEra: SessionIndex = prod_or_fast!(6, 2);
	/// Duration of a relay session in our blocks. Needs to be hardcoded per-runtime.
	pub const RelaySessionDuration: BlockNumber = 1 * HOURS;
	// 2 eras for unbonding (12 hours).
	pub const BondingDuration: sp_staking::EraIndex = 2;
	// 1 era in which slashes can be cancelled (6 hours).
	pub const SlashDeferDuration: sp_staking::EraIndex = 1;
	pub const MaxControllersInDeprecationBatch: u32 = 751;
	// alias for 16, which is the max nominations per nominator in the runtime.
	pub const MaxNominations: u32 = <NposCompactSolution16 as frame_election_provider_support::NposSolution>::LIMIT as u32;
	pub storage PlanningEraOffset: u32 = prod_or_fast!(2, 1);
	pub const MaxEraDuration: u64 = RelaySessionDuration::get() as u64 * RELAY_CHAIN_SLOT_DURATION_MILLIS as u64 * SessionsPerEra::get() as u64;
}

impl pallet_staking_async::Config for Runtime {
	type Filter = ();
	type OldCurrency = Balances;
	type Currency = Balances;
	type CurrencyBalance = Balance;
	type RuntimeHoldReason = RuntimeHoldReason;
	type CurrencyToVote = sp_staking::currency_to_vote::SaturatingCurrencyToVote;
	type RewardRemainder = ();
	type Slash = ();
	type Reward = ();
	type SessionsPerEra = SessionsPerEra;
	type BondingDuration = BondingDuration;
	type SlashDeferDuration = SlashDeferDuration;
	type AdminOrigin = EitherOf<EnsureRoot<AccountId>, StakingAdmin>;
	type EraPayout = EraPayout;
	type MaxExposurePageSize = MaxExposurePageSize;
	type ElectionProvider = MultiBlockElection;
	type VoterList = VoterList;
	type TargetList = UseValidatorsMap<Self>;
	type MaxValidatorSet = MaxValidatorSet;
	type NominationsQuota = pallet_staking_async::FixedNominationsQuota<{ MaxNominations::get() }>;
	type MaxUnlockingChunks = frame_support::traits::ConstU32<32>;
	type HistoryDepth = frame_support::traits::ConstU32<84>;
	type MaxControllersInDeprecationBatch = MaxControllersInDeprecationBatch;
	type EventListeners = (NominationPools, DelegatedStaking);
	type WeightInfo = ();
	type MaxInvulnerables = frame_support::traits::ConstU32<20>;
	type MaxDisabledValidators = ConstU32<100>;
	type PlanningEraOffset = PlanningEraOffset;
	type RcClientInterface = StakingRcClient;
	type MaxEraDuration = MaxEraDuration;
}

impl pallet_staking_async_rc_client::Config for Runtime {
	type RelayChainOrigin = EnsureRoot<AccountId>;
	type AHStakingInterface = Staking;
	type SendToRelayChain = StakingXcmToRelayChain;
}

#[derive(Encode, Decode)]
// Call indices taken from westend-next runtime.
pub enum RelayChainRuntimePallets {
	// Audit: index of `AssetHubStakingClient` in westend.
	#[codec(index = 67)]
	AhClient(AhClientCalls),
}

#[derive(Encode, Decode)]
pub enum AhClientCalls {
	// index of `fn validator_set` in `staking-async-ah-client`. It has only one call.
	#[codec(index = 0)]
	ValidatorSet(rc_client::ValidatorSetReport<AccountId>),
}

pub struct ValidatorSetToXcm;
impl sp_runtime::traits::Convert<rc_client::ValidatorSetReport<AccountId>, Xcm<()>>
	for ValidatorSetToXcm
{
	fn convert(report: rc_client::ValidatorSetReport<AccountId>) -> Xcm<()> {
		Xcm(vec![
			Instruction::UnpaidExecution {
				weight_limit: WeightLimit::Unlimited,
				check_origin: None,
			},
			Instruction::Transact {
				origin_kind: OriginKind::Native,
				fallback_max_weight: None,
				call: RelayChainRuntimePallets::AhClient(AhClientCalls::ValidatorSet(report))
					.encode()
					.into(),
			},
		])
	}
}

parameter_types! {
	pub RelayLocation: Location = Location::parent();
}

pub struct StakingXcmToRelayChain;

impl rc_client::SendToRelayChain for StakingXcmToRelayChain {
	type AccountId = AccountId;
	fn validator_set(report: rc_client::ValidatorSetReport<Self::AccountId>) {
		rc_client::XCMSender::<
			xcm_config::XcmRouter,
			RelayLocation,
			rc_client::ValidatorSetReport<Self::AccountId>,
			ValidatorSetToXcm,
		>::split_then_send(report, Some(8));
	}
}

impl pallet_fast_unstake::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type BatchSize = ConstU32<64>;
	type Deposit = ConstU128<{ UNITS }>;
	type ControlOrigin = EnsureRoot<AccountId>;
	type Staking = Staking;
	type MaxErasToCheckPerBlock = ConstU32<1>;
	type WeightInfo = weights::pallet_fast_unstake::WeightInfo<Runtime>;
}

parameter_types! {
	pub const PoolsPalletId: PalletId = PalletId(*b"py/nopls");
	pub const MaxPointsToBalance: u8 = 10;
}

impl pallet_nomination_pools::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = weights::pallet_nomination_pools::WeightInfo<Self>;
	type Currency = Balances;
	type RuntimeFreezeReason = RuntimeFreezeReason;
	type RewardCounter = FixedU128;
	type BalanceToU256 = BalanceToU256;
	type U256ToBalance = U256ToBalance;
	type StakeAdapter =
		pallet_nomination_pools::adapter::DelegateStake<Self, Staking, DelegatedStaking>;
	type PostUnbondingPoolsWindow = ConstU32<4>;
	type MaxMetadataLen = ConstU32<256>;
	// we use the same number of allowed unlocking chunks as with staking.
	type MaxUnbonding = <Self as pallet_staking_async::Config>::MaxUnlockingChunks;
	type PalletId = PoolsPalletId;
	type MaxPointsToBalance = MaxPointsToBalance;
	type AdminOrigin = EitherOf<EnsureRoot<AccountId>, StakingAdmin>;
	type BlockNumberProvider = RelaychainDataProvider<Runtime>;
	type Filter = Nothing;
}

parameter_types! {
	pub const DelegatedStakingPalletId: PalletId = PalletId(*b"py/dlstk");
	pub const SlashRewardFraction: Perbill = Perbill::from_percent(1);
}

impl pallet_delegated_staking::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type PalletId = DelegatedStakingPalletId;
	type Currency = Balances;
	type OnSlash = ();
	type SlashRewardFraction = SlashRewardFraction;
	type RuntimeHoldReason = RuntimeHoldReason;
	type CoreStaking = Staking;
}

/// The payload being signed in transactions.
pub type SignedPayload = generic::SignedPayload<RuntimeCall, TxExtension>;
/// Unchecked extrinsic type as expected by this runtime.
pub type UncheckedExtrinsic =
	generic::UncheckedExtrinsic<Address, RuntimeCall, Signature, TxExtension>;

impl frame_system::offchain::SigningTypes for Runtime {
	type Public = <Signature as Verify>::Signer;
	type Signature = Signature;
}

impl<C> frame_system::offchain::CreateTransactionBase<C> for Runtime
where
	RuntimeCall: From<C>,
{
	type RuntimeCall = RuntimeCall;
	type Extrinsic = UncheckedExtrinsic;
}

impl<LocalCall> frame_system::offchain::CreateTransaction<LocalCall> for Runtime
where
	RuntimeCall: From<LocalCall>,
{
	type Extension = TxExtension;

	fn create_transaction(call: RuntimeCall, extension: TxExtension) -> UncheckedExtrinsic {
		UncheckedExtrinsic::new_transaction(call, extension)
	}
}

/// Submits a transaction with the node's public and signature type. Adheres to the signed extension
/// format of the chain.
impl<LocalCall> frame_system::offchain::CreateSignedTransaction<LocalCall> for Runtime
where
	RuntimeCall: From<LocalCall>,
{
	fn create_signed_transaction<
		C: frame_system::offchain::AppCrypto<Self::Public, Self::Signature>,
	>(
		call: RuntimeCall,
		public: <Signature as Verify>::Signer,
		account: AccountId,
		nonce: <Runtime as frame_system::Config>::Nonce,
	) -> Option<UncheckedExtrinsic> {
		use sp_runtime::traits::StaticLookup;
		// take the biggest period possible.
		let period =
			BlockHashCount::get().checked_next_power_of_two().map(|c| c / 2).unwrap_or(2) as u64;

		let current_block = System::block_number()
			.saturated_into::<u64>()
			// The `System::block_number` is initialized with `n+1`,
			// so the actual block number is `n`.
			.saturating_sub(1);
		let tip = 0;
		let tx_ext = TxExtension::from((
			frame_system::AuthorizeCall::<Runtime>::new(),
			frame_system::CheckNonZeroSender::<Runtime>::new(),
			frame_system::CheckSpecVersion::<Runtime>::new(),
			frame_system::CheckTxVersion::<Runtime>::new(),
			frame_system::CheckGenesis::<Runtime>::new(),
			frame_system::CheckEra::<Runtime>::from(generic::Era::mortal(period, current_block)),
			frame_system::CheckNonce::<Runtime>::from(nonce),
			frame_system::CheckWeight::<Runtime>::new(),
			pallet_asset_conversion_tx_payment::ChargeAssetTxPayment::<Runtime>::from(tip, None),
			frame_metadata_hash_extension::CheckMetadataHash::<Runtime>::new(true),
		));
		let raw_payload = SignedPayload::new(call, tx_ext)
			.map_err(|e| {
				log::warn!("Unable to create signed payload: {:?}", e);
			})
			.ok()?;
		let signature = raw_payload.using_encoded(|payload| C::sign(payload, public))?;
		let (call, tx_ext, _) = raw_payload.deconstruct();
		let address = <Runtime as frame_system::Config>::Lookup::unlookup(account);
		let transaction = UncheckedExtrinsic::new_signed(call, address, signature, tx_ext);
		Some(transaction)
	}
}

impl<LocalCall> frame_system::offchain::CreateInherent<LocalCall> for Runtime
where
	RuntimeCall: From<LocalCall>,
{
	fn create_bare(call: RuntimeCall) -> UncheckedExtrinsic {
		UncheckedExtrinsic::new_bare(call)
	}
}
