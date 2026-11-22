use super::{DocumentKey, principal};
use crate::{
    config::SourceConfig,
    db::Db,
    documents,
    embed::{Embedder, Model, Store, cosine},
    hybrid::NAME_ONLY_LIMIT,
    search::DocIndex,
};
use anyhow::{Result, ensure};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Ranked {
    pub key: DocumentKey,
    pub name_only: bool,
}

pub(super) struct Semantic {
    encoder: Box<dyn Embedder>,
    prefix: String,
    dim: usize,
    store: Store,
    entries: Vec<Option<i64>>,
    documents: HashMap<i64, Ranked>,
}

impl Semantic {
    pub async fn load(
        data: &Path,
        model: &Model,
        index: &DocIndex,
        db: &Db,
        sources: &[SourceConfig],
        open_encoder: impl FnOnce() -> Result<Box<dyn Embedder>>,
    ) -> Result<Self> {
        let (store, entries) = Store::load_current(&model.store_dir(data), model, index)?;
        let user = principal();
        let mut ids: Vec<_> = entries.iter().map(|entry| entry.doc_id).collect();
        ids.sort_unstable();
        ids.dedup();
        let names: HashSet<_> = sources.iter().map(|s| s.name.as_str()).collect();
        let mut rows = HashMap::new();
        for batch in ids.chunks(500) {
            rows.extend(
                documents::rows(db, index.table(), batch.to_vec())
                    .await
                    .map_err(|_| anyhow::anyhow!("Status dokumen untuk evaluasi gagal dibaca."))?,
            );
        }
        // Like search_ranked with the eval admin, check the index source plus the
        // current source config and matching status row. Index labels all allow admin.
        let mut documents = HashMap::new();
        let entries = entries
            .into_iter()
            .map(|entry| {
                let row = rows.get(&entry.doc_id).filter(|row| {
                    names.contains(entry.source.as_str())
                        && row.allows_entry(&user, sources)
                        && row.name_only() == entry.name_only
                })?;
                documents.entry(row.id).or_insert_with(|| Ranked {
                    key: (row.source.clone(), row.rel_path.clone()),
                    name_only: row.name_only(),
                });
                Some(row.id)
            })
            .collect();
        Ok(Self {
            encoder: open_encoder()?,
            prefix: model.query_prefix.clone(),
            dim: model.dim,
            store,
            entries,
            documents,
        })
    }

    pub fn rank(&mut self, question: &str, limit: usize) -> Result<Vec<Ranked>> {
        let queries = self
            .encoder
            .embed(&[format!("{}{question}", self.prefix)])?;
        ensure!(
            queries.len() == 1
                && queries[0].len() == self.dim
                && (cosine(&queries[0], &queries[0]) - 1.0).abs() < 0.001,
            "Vektor pertanyaan tidak valid."
        );
        let mut best = HashMap::<_, f32>::new();
        for (id, score) in self.entries.iter().zip(self.store.scores(&queries[0])) {
            if let Some(id) = id {
                best.entry(*id)
                    .and_modify(|best| *best = best.max(score))
                    .or_insert(score);
            }
        }
        let mut scores: Vec<_> = best.into_iter().collect();
        scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(cap(
            scores
                .into_iter()
                .map(|(id, _)| self.documents[&id].clone()),
            limit,
        ))
    }
}

fn cap(items: impl IntoIterator<Item = Ranked>, limit: usize) -> Vec<Ranked> {
    let mut name_only = 0;
    items
        .into_iter()
        .filter(|doc| {
            name_only += usize::from(doc.name_only);
            !doc.name_only || name_only <= NAME_ONLY_LIMIT
        })
        .take(limit)
        .collect()
}
