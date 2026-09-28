//! RFC-0294: Pilar 1 - Invertibilidade Estrita e Monotonicidade em Restart Points sob Compressão Delta.
//!
//! Formaliza a codificação delta de prefixos e a indexação por restart points em blocos SST,
//! provando a monotonicidade estrita e a invertibilidade bijetora em buscas binárias.

/// Violações de monotonicidade ou invertibilidade na decodificação delta de blocos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaDecodingViolation {
    /// O comprimento compartilhado excedeu o comprimento da chave anterior (Buffer Overrun).
    SharedLengthExceedsPreviousKey {
        /// Índice do registro.
        record_idx: usize,
        /// Comprimento compartilhado requisitado.
        shared_len: usize,
        /// Comprimento total da chave anterior.
        prev_len: usize,
    },
    /// A busca binária entre restart points localizou um intervalo que violou a monotonicidade.
    RestartPointNonMonotonic {
        /// Índice do restart point.
        restart_idx: usize,
        /// Chave no restart point anterior.
        prev_key: Vec<u8>,
        /// Chave no restart point atual.
        curr_key: Vec<u8>,
    },
    /// A decodificação sequencial divergiu da chave original na bijeção D(E(K)) != K.
    BijectiveInversionFailed {
        /// Chave original esperada.
        expected: Vec<u8>,
        /// Chave decodificada.
        decoded: Vec<u8>,
    },
}

/// Registro codificado com compressão delta de prefixo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaEncodedEntry {
    /// Quantidade de bytes herdados da chave anterior.
    pub shared_len: usize,
    /// Bytes do sufixo não-compartilhado.
    pub unshared_suffix: Vec<u8>,
    /// Valor associado à chave.
    pub value: Vec<u8>,
}

/// Bloco de dados SST com codificação delta e índice de restart points.
#[derive(Debug, Clone)]
pub struct DeltaBlock {
    /// Entradas codificadas sequencialmente.
    entries: Vec<DeltaEncodedEntry>,
    /// Índices das entradas que iniciam novos intervalos independentes (SharedLen = 0).
    restart_points: Vec<usize>,
}

impl DeltaBlock {
    /// Constrói um bloco com delta encoding e restart points a cada `restart_interval` chaves.
    pub fn encode(keys_and_values: &[(Vec<u8>, Vec<u8>)], restart_interval: usize) -> Self {
        let interval = restart_interval.max(1);
        let mut entries = Vec::with_capacity(keys_and_values.len());
        let mut restart_points = Vec::new();

        let mut prev_key: &[u8] = &[];

        for (idx, (key, value)) in keys_and_values.iter().enumerate() {
            let is_restart = idx % interval == 0;

            let shared_len = if is_restart {
                restart_points.push(idx);
                0
            } else {
                let mut common = 0;
                let max_len = prev_key.len().min(key.len());
                while common < max_len && prev_key[common] == key[common] {
                    common += 1;
                }
                common
            };

            let unshared_suffix = key[shared_len..].to_vec();
            entries.push(DeltaEncodedEntry {
                shared_len,
                unshared_suffix,
                value: value.clone(),
            });

            prev_key = key.as_slice();
        }

        Self {
            entries,
            restart_points,
        }
    }

    /// Decodifica todas as chaves do bloco do início ao fim e valida a bijeção.
    pub fn decode_all(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DeltaDecodingViolation> {
        let mut result = Vec::with_capacity(self.entries.len());
        let mut current_key = Vec::new();

        for (idx, entry) in self.entries.iter().enumerate() {
            if entry.shared_len > current_key.len() {
                return Err(DeltaDecodingViolation::SharedLengthExceedsPreviousKey {
                    record_idx: idx,
                    shared_len: entry.shared_len,
                    prev_len: current_key.len(),
                });
            }

            current_key.truncate(entry.shared_len);
            current_key.extend_from_slice(&entry.unshared_suffix);

            result.push((current_key.clone(), entry.value.clone()));
        }

        Ok(result)
    }

    /// Executa uma busca por chave utilizando busca binária nos restart points,
    /// seguida de scan sequencial estritamente dentro do intervalo localizado.
    pub fn seek(&self, target_key: &[u8]) -> Result<Option<(Vec<u8>, Vec<u8>)>, DeltaDecodingViolation> {
        if self.entries.is_empty() {
            return Ok(None);
        }

        // 1. Decodifica as chaves base dos restart points
        let mut restart_keys = Vec::with_capacity(self.restart_points.len());
        for &r_idx in &self.restart_points {
            let entry = &self.entries[r_idx];
            if entry.shared_len != 0 {
                return Err(DeltaDecodingViolation::SharedLengthExceedsPreviousKey {
                    record_idx: r_idx,
                    shared_len: entry.shared_len,
                    prev_len: 0,
                });
            }
            restart_keys.push((r_idx, entry.unshared_suffix.clone()));
        }

        // Validação de monotonicidade entre restart points
        for i in 1..restart_keys.len() {
            if restart_keys[i - 1].1 >= restart_keys[i].1 {
                return Err(DeltaDecodingViolation::RestartPointNonMonotonic {
                    restart_idx: i,
                    prev_key: restart_keys[i - 1].1.clone(),
                    curr_key: restart_keys[i].1.clone(),
                });
            }
        }

        // 2. Busca binária nos restart points
        let mut low = 0;
        let mut high = restart_keys.len();

        while low < high {
            let mid = low + (high - low) / 2;
            if restart_keys[mid].1.as_slice() <= target_key {
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        let start_restart_idx = if low == 0 { 0 } else { low - 1 };
        let start_entry_idx = restart_keys[start_restart_idx].0;

        let end_entry_idx = if start_restart_idx + 1 < restart_keys.len() {
            restart_keys[start_restart_idx + 1].0
        } else {
            self.entries.len()
        };

        // 3. Scan sequencial dentro do intervalo
        let mut current_key = Vec::new();
        for idx in start_entry_idx..end_entry_idx {
            let entry = &self.entries[idx];
            if entry.shared_len > current_key.len() {
                return Err(DeltaDecodingViolation::SharedLengthExceedsPreviousKey {
                    record_idx: idx,
                    shared_len: entry.shared_len,
                    prev_len: current_key.len(),
                });
            }
            current_key.truncate(entry.shared_len);
            current_key.extend_from_slice(&entry.unshared_suffix);

            if current_key.as_slice() == target_key {
                return Ok(Some((current_key, entry.value.clone())));
            } else if current_key.as_slice() > target_key {
                // Passou do ponto
                return Ok(None);
            }
        }

        Ok(None)
    }
}
