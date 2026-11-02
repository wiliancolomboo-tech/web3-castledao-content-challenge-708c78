# Initialize local Solana test validator configured to localnet
solana config set --url localhost

# Generate program deployer keypair if not present
solana-keygen new --no-bip39-passphrase --outfile ~/.config/solana/id.json -f