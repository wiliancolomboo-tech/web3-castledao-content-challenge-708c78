# Set Solana CLI config to Devnet
solana config set --url https://api.devnet.solana.com

# Airdrop test SOL to your deployment wallet
solana airdrop 2

# Deploy the binary
anchor deploy --provider.cluster devnet