# Configure environment to Devnet
solana config set --url devnet

# Airdrop SOL for deployment fees
solana airdrop 2

# Deploy program
anchor deploy --provider.cluster devnet