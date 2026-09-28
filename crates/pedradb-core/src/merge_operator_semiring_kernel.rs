//! RFC-0291 Fronteira 6: Álgebra de Semianéis de Merge Operators e Não-Comutatividade de Aniquiladores.
//!
//! Formaliza axiomaticamente operadores de merge como um semianel não-comutativo
//! com absorção à esquerda, garantindo confluência estrita entre coalescência em tempo
//! de leitura (read-time merge) e em tempo de compactação (compaction-time merge).

#![forbid(unsafe_code)]

/// Mutação atômica em um nível da LSM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LsmRecordMutation {
    Put(Vec<u8>),
    Delete,
    Merge(Vec<u8>),
}

/// Função de fusão de operandos definindo um semigrupo associativo.
pub type MergeFoldFn = fn(existing: Option<&[u8]>, operand: &[u8]) -> Vec<u8>;
pub type OperandCombineFn = fn(op1: &[u8], op2: &[u8]) -> Vec<u8>;

/// Violação da álgebra de semianéis do operador de merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeSemiringViolation {
    NonAssociativeOperandFold {
        op1: Vec<u8>,
        op2: Vec<u8>,
        op3: Vec<u8>,
        left_fold: Vec<u8>,
        right_fold: Vec<u8>,
    },
    ReadCompactionDivergence {
        read_result: Option<Vec<u8>>,
        compaction_result: Option<Vec<u8>>,
    },
}

/// Oráculo e avaliador algébrico de Merge Operators sob Semianéis.
pub struct MergeOperatorSemiringEvaluator {
    pub fold_fn: MergeFoldFn,
    pub combine_fn: OperandCombineFn,
}

impl MergeOperatorSemiringEvaluator {
    pub const fn new(fold_fn: MergeFoldFn, combine_fn: OperandCombineFn) -> Self {
        Self {
            fold_fn,
            combine_fn,
        }
    }

    /// Avaliação sequencial em tempo de leitura (read-time fold do mais recente ao mais antigo).
    ///
    /// `mutations` ordenados do mais recente (topo da pilha) ao mais antigo (base).
    #[must_use]
    pub fn evaluate_read_time(&self, mutations_newest_to_oldest: &[LsmRecordMutation]) -> Option<Vec<u8>> {
        let mut operands = Vec::new();

        for m in mutations_newest_to_oldest {
            match m {
                LsmRecordMutation::Merge(val) => {
                    operands.push(val.as_slice());
                }
                LsmRecordMutation::Put(base_val) => {
                    // Encontrou valor base definitivo (aniquilador de merge à esquerda):
                    // Aplica operandos acumulados do mais antigo ao mais recente sobre a base
                    let mut current = base_val.clone();
                    for &op in operands.iter().rev() {
                        current = (self.fold_fn)(Some(&current), op);
                    }
                    return Some(current);
                }
                LsmRecordMutation::Delete => {
                    // Encontrou tombstone (aniquilador nulo):
                    // Aplica operandos acumulados a partir do estado vazio
                    if operands.is_empty() {
                        return None;
                    }
                    let mut current = (self.fold_fn)(None, operands[operands.len() - 1]);
                    for &op in operands[..operands.len() - 1].iter().rev() {
                        current = (self.fold_fn)(Some(&current), op);
                    }
                    return Some(current);
                }
            }
        }

        // Se só existirem operandos de merge até a raiz
        if operands.is_empty() {
            None
        } else {
            let mut current = (self.fold_fn)(None, operands[operands.len() - 1]);
            for &op in operands[..operands.len() - 1].iter().rev() {
                current = (self.fold_fn)(Some(&current), op);
            }
            Some(current)
        }
    }

    /// Coalescência em tempo de compactação (agrupa operandos adjacentes em um novo Merge ou Put).
    #[must_use]
    pub fn compact_slice(&self, mutations_newest_to_oldest: &[LsmRecordMutation]) -> Vec<LsmRecordMutation> {
        let mut compacted = Vec::new();
        let mut accumulated_merge: Option<Vec<u8>> = None;

        for m in mutations_newest_to_oldest {
            match m {
                LsmRecordMutation::Merge(op) => {
                    if let Some(prev) = accumulated_merge.take() {
                        // Combina: prev era mais novo que op
                        accumulated_merge = Some((self.combine_fn)(op, &prev));
                    } else {
                        accumulated_merge = Some(op.clone());
                    }
                }
                LsmRecordMutation::Put(base) => {
                    if let Some(merged_op) = accumulated_merge.take() {
                        let final_val = (self.fold_fn)(Some(base), &merged_op);
                        compacted.push(LsmRecordMutation::Put(final_val));
                    } else {
                        compacted.push(LsmRecordMutation::Put(base.clone()));
                    }
                }
                LsmRecordMutation::Delete => {
                    if let Some(merged_op) = accumulated_merge.take() {
                        let final_val = (self.fold_fn)(None, &merged_op);
                        compacted.push(LsmRecordMutation::Put(final_val));
                    } else {
                        compacted.push(LsmRecordMutation::Delete);
                    }
                }
            }
        }

        if let Some(leftover_op) = accumulated_merge {
            compacted.push(LsmRecordMutation::Merge(leftover_op));
        }

        compacted
    }

    /// Verifica formalmente o Teorema da Associatividade e Confluência:
    /// $\text{EvaluateReadTime}(M) \equiv \text{EvaluateReadTime}(\text{CompactSlice}(M))$
    pub fn verify_confluence(
        &self,
        mutations: &[LsmRecordMutation],
    ) -> Result<(), MergeSemiringViolation> {
        let read_val = self.evaluate_read_time(mutations);
        let compacted_slice = self.compact_slice(mutations);
        let compacted_val = self.evaluate_read_time(&compacted_slice);

        if read_val != compacted_val {
            return Err(MergeSemiringViolation::ReadCompactionDivergence {
                read_result: read_val,
                compaction_result: compacted_val,
            });
        }
        Ok(())
    }

    /// Verifica axioma de associatividade de operandos puros: $(a \oplus b) \oplus c = a \oplus (b \oplus c)$.
    pub fn verify_associativity(
        &self,
        op1: &[u8],
        op2: &[u8],
        op3: &[u8],
    ) -> Result<(), MergeSemiringViolation> {
        let left = (self.combine_fn)(&(self.combine_fn)(op1, op2), op3);
        let right = (self.combine_fn)(op1, &(self.combine_fn)(op2, op3));

        if left != right {
            return Err(MergeSemiringViolation::NonAssociativeOperandFold {
                op1: op1.to_vec(),
                op2: op2.to_vec(),
                op3: op3.to_vec(),
                left_fold: left,
                right_fold: right,
            });
        }
        Ok(())
    }
}
