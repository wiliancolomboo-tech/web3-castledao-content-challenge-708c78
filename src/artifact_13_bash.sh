# Configure CLI to Devnet
solana config set --url https://api.devnet.solana.com

# Airdrop SOL if balance is low
solana airdrop 2

# Deploy
anchor deploy --provider.cluster devnet