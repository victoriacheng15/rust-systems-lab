use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use nom::IResult;
use nom::branch::alt;
use nom::bytes::complete::{tag_no_case, take_until, take_while1};
use nom::character::complete::{char, multispace0, multispace1};
use nom::combinator::{all_consuming, map, map_res, opt};
use nom::sequence::{delimited, preceded, separated_pair, terminated, tuple};
use std::io::{self, Write};

const LEAF_CAPACITY: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    id: u64,
    value: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct RowRef(usize);

#[derive(Debug, Default)]
struct RowArena {
    rows: Vec<Row>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LeafPage {
    keys: Vec<u64>,
    values: Vec<RowRef>,
}

#[derive(Debug, Default)]
struct PrimaryIndex {
    leaves: Vec<LeafPage>,
}

#[derive(Debug, Default)]
struct Table {
    arena: RowArena,
    index: PrimaryIndex,
}

#[derive(Debug, PartialEq, Eq)]
enum Statement {
    Insert { id: u64, value: String },
    Select { id: u64 },
    Scan { start: u64, end: u64 },
}

#[derive(Parser)]
#[command(author, version, about = "Embedded in-memory query engine", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a single query against an empty in-memory table
    Query { sql: String },
    /// Start an interactive session backed by one in-memory table
    Repl,
}

impl RowArena {
    fn insert(&mut self, row: Row) -> RowRef {
        let reference = RowRef(self.rows.len());
        self.rows.push(row);
        reference
    }

    fn get(&self, reference: RowRef) -> Option<&Row> {
        self.rows.get(reference.0)
    }
}

impl PrimaryIndex {
    fn insert(&mut self, key: u64, row: RowRef) -> Result<()> {
        if self.leaves.is_empty() {
            self.leaves.push(LeafPage::default());
        }

        let leaf_index = self.find_leaf_index(key);
        let leaf = &mut self.leaves[leaf_index];
        match leaf.keys.binary_search(&key) {
            Ok(_) => return Err(anyhow!("duplicate primary key: {key}")),
            Err(position) => {
                leaf.keys.insert(position, key);
                leaf.values.insert(position, row);
            }
        }

        if self.leaves[leaf_index].keys.len() > LEAF_CAPACITY {
            self.split_leaf(leaf_index);
        }

        Ok(())
    }

    fn get(&self, key: u64) -> Option<RowRef> {
        let leaf = self.leaves.get(self.find_leaf_index(key))?;
        leaf.keys
            .binary_search(&key)
            .ok()
            .map(|position| leaf.values[position])
    }

    fn range(&self, start: u64, end: u64) -> Vec<RowRef> {
        if start > end {
            return Vec::new();
        }

        self.leaves
            .iter()
            .flat_map(|leaf| {
                leaf.keys
                    .iter()
                    .zip(leaf.values.iter())
                    .filter_map(|(key, row)| ((*key >= start) && (*key <= end)).then_some(*row))
            })
            .collect()
    }

    fn find_leaf_index(&self, key: u64) -> usize {
        let Some(index) = self
            .leaves
            .iter()
            .position(|leaf| leaf.keys.last().is_none_or(|last_key| key <= *last_key))
        else {
            return self.leaves.len().saturating_sub(1);
        };

        index
    }

    fn split_leaf(&mut self, leaf_index: usize) {
        let leaf = &mut self.leaves[leaf_index];
        let split_at = leaf.keys.len() / 2;
        let sibling = LeafPage {
            keys: leaf.keys.split_off(split_at),
            values: leaf.values.split_off(split_at),
        };
        self.leaves.insert(leaf_index + 1, sibling);
    }
}

impl Table {
    fn execute(&mut self, statement: Statement) -> Result<Vec<Row>> {
        match statement {
            Statement::Insert { id, value } => {
                let row_ref = self.arena.insert(Row { id, value });
                if let Err(error) = self.index.insert(id, row_ref) {
                    self.arena.rows.pop();
                    return Err(error);
                }
                Ok(vec![
                    self.arena
                        .get(row_ref)
                        .expect("inserted row exists")
                        .clone(),
                ])
            }
            Statement::Select { id } => Ok(self
                .index
                .get(id)
                .and_then(|row_ref| self.arena.get(row_ref))
                .cloned()
                .into_iter()
                .collect()),
            Statement::Scan { start, end } => Ok(self
                .index
                .range(start, end)
                .into_iter()
                .filter_map(|row_ref| self.arena.get(row_ref).cloned())
                .collect()),
        }
    }
}

fn parse_statement(input: &str) -> Result<Statement> {
    let (_, statement) = all_consuming(terminated(
        delimited(
            multispace0,
            alt((parse_insert, parse_select, parse_scan)),
            multispace0,
        ),
        opt(char(';')),
    ))(input)
    .map_err(|error| anyhow!("invalid query: {error:?}"))?;

    Ok(statement)
}

fn parse_insert(input: &str) -> IResult<&str, Statement> {
    map(
        tuple((
            tag_no_case("insert"),
            multispace1,
            unsigned_integer,
            multispace1,
            quoted_string,
        )),
        |(_, _, id, _, value)| Statement::Insert { id, value },
    )(input)
}

fn parse_select(input: &str) -> IResult<&str, Statement> {
    map(
        tuple((tag_no_case("select"), multispace1, unsigned_integer)),
        |(_, _, id)| Statement::Select { id },
    )(input)
}

fn parse_scan(input: &str) -> IResult<&str, Statement> {
    map(
        tuple((
            tag_no_case("scan"),
            multispace1,
            separated_pair(
                unsigned_integer,
                preceded(multispace0, char('.')),
                preceded(char('.'), unsigned_integer),
            ),
        )),
        |(_, _, (start, end))| Statement::Scan { start, end },
    )(input)
}

fn quoted_string(input: &str) -> IResult<&str, String> {
    map(
        delimited(char('\''), take_until("'"), char('\'')),
        ToString::to_string,
    )(input)
}

fn unsigned_integer(input: &str) -> IResult<&str, u64> {
    map_res(take_while1(|c: char| c.is_ascii_digit()), str::parse)(input)
}

fn print_rows(rows: &[Row]) {
    if rows.is_empty() {
        println!("No rows");
        return;
    }

    for row in rows {
        println!("{} {}", row.id, row.value);
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut table = Table::default();

    match cli.command {
        Commands::Query { sql } => {
            let statement = parse_statement(&sql)?;
            let rows = table.execute(statement)?;
            print_rows(&rows);
        }
        Commands::Repl => run_repl(&mut table)?,
    }

    Ok(())
}

fn run_repl(table: &mut Table) -> Result<()> {
    let stdin = io::stdin();
    let mut line = String::new();

    loop {
        print!("query> ");
        io::stdout().flush()?;

        line.clear();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }

        let input = line.trim();
        if input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit") {
            break;
        }

        if input.is_empty() {
            continue;
        }

        match parse_statement(input).and_then(|statement| table.execute(statement)) {
            Ok(rows) => print_rows(&rows),
            Err(error) => eprintln!("{error}"),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_accepts_insert_select_and_scan() -> Result<()> {
        assert_eq!(
            parse_statement("insert 42 'meaning'")?,
            Statement::Insert {
                id: 42,
                value: "meaning".to_string(),
            }
        );
        assert_eq!(parse_statement("select 42;")?, Statement::Select { id: 42 });
        assert_eq!(
            parse_statement("scan 10..20")?,
            Statement::Scan { start: 10, end: 20 }
        );
        Ok(())
    }

    #[test]
    fn index_splits_leaves_and_preserves_ordered_range_scan() -> Result<()> {
        let mut table = Table::default();
        for id in [9, 1, 5, 3, 7, 2, 8] {
            table.execute(Statement::Insert {
                id,
                value: format!("row-{id}"),
            })?;
        }

        assert!(table.index.leaves.len() > 1);
        let rows = table.execute(Statement::Scan { start: 2, end: 7 })?;
        let ids = rows.into_iter().map(|row| row.id).collect::<Vec<_>>();
        assert_eq!(ids, vec![2, 3, 5, 7]);
        Ok(())
    }

    #[test]
    fn select_uses_primary_key_lookup() -> Result<()> {
        let mut table = Table::default();
        table.execute(Statement::Insert {
            id: 12,
            value: "alpha".to_string(),
        })?;

        let rows = table.execute(Statement::Select { id: 12 })?;
        assert_eq!(
            rows,
            vec![Row {
                id: 12,
                value: "alpha".to_string(),
            }]
        );
        assert!(table.execute(Statement::Select { id: 99 })?.is_empty());
        Ok(())
    }

    #[test]
    fn duplicate_primary_keys_are_rejected() -> Result<()> {
        let mut table = Table::default();
        table.execute(Statement::Insert {
            id: 1,
            value: "first".to_string(),
        })?;

        let error = table
            .execute(Statement::Insert {
                id: 1,
                value: "second".to_string(),
            })
            .expect_err("duplicate key should fail");

        assert!(error.to_string().contains("duplicate primary key"));
        assert_eq!(table.arena.rows.len(), 1);
        Ok(())
    }
}
