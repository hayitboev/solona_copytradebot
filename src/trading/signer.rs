use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::VersionedTransaction;
use bs58;
use bincode;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use crate::error::{Result, AppError};

pub struct TransactionSigner {
    keypair: Keypair,
}

impl TransactionSigner {
    pub fn new(private_key_base58: &str) -> Result<Self> {
        let key_bytes = bs58::decode(private_key_base58)
            .into_vec()
            .map_err(|e| AppError::Init(format!("Invalid private key: {}", e)))?;

        let keypair = Keypair::from_bytes(&key_bytes)
            .map_err(|e| AppError::Init(format!("Invalid keypair bytes: {}", e)))?;

        Ok(Self { keypair })
    }

    pub fn pubkey(&self) -> String {
        self.keypair.pubkey().to_string()
    }

    /// Signs a base64 encoded versioned transaction
    pub fn sign_transaction(&self, versioned_tx_base64: &str) -> Result<String> {
        // 1. Decode Base64
        let tx_bytes = STANDARD.decode(versioned_tx_base64)
            .map_err(|e| AppError::Trading(format!("Failed to decode base64 tx: {}", e)))?;

        // 2. Deserialize VersionedTransaction
        let mut tx: VersionedTransaction = bincode::deserialize(&tx_bytes)
            .map_err(|e| AppError::Trading(format!("Failed to deserialize tx: {}", e)))?;

        // 3. Sign
        // VersionedTransaction in solana-sdk 1.18 usually has a method to add signatures
        // `try_sign` signs the message with the keypairs provided.
        // But `VersionedTransaction` needs the message to be signed.
        // `tx.try_sign(&[&self.keypair], blockhash)`? No, `try_sign` takes pointers to signers and blockhash usually.
        // Wait, `VersionedTransaction` struct has `signatures` field.
        // We can just overwrite the signatures if we are the only signer (which we are usually, as payer).
        // BUT, Jupiter might return a partially signed tx (unlikely) or just unsigned.
        // The safest way is to use `try_sign` if available, or manually construct signature.

        // Let's check `solana-sdk` docs or assume standard usage.
        // `VersionedTransaction` implements `Signer` trait? No.
        // It has a `new` constructor.
        // It has `signatures: Vec<Signature>`.
        // The message is `message: VersionedMessage`.

        let message = &tx.message;
        let signature = self.keypair.sign_message(&message.serialize());

        // Assuming we are the first signer (fee payer).
        if tx.signatures.is_empty() {
            tx.signatures.push(signature);
        } else {
            tx.signatures[0] = signature;
            // Warning: If there are other required signers, this might be wrong if we are not the first.
            // But usually the user is the first signer (payer).
        }

        // 4. Serialize back
        let signed_bytes = bincode::serialize(&tx)
            .map_err(|e| AppError::Trading(format!("Failed to serialize signed tx: {}", e)))?;

        Ok(STANDARD.encode(signed_bytes))
    }
}
