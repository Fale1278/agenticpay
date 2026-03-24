#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, String, Vec};

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ProjectStatus {
    Created,
    Funded,
    InProgress,
    WorkSubmitted,
    Verified,
    Completed,
    Disputed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Project {
    pub id: u64,
    pub client: Address,
    pub freelancer: Address,
    pub amount: i128,
    pub deposited: i128,
    pub status: ProjectStatus,
    pub github_repo: String,
    pub description: String,
    pub created_at: u64,
}

#[contracttype]
pub enum DataKey {
    Project(u64),
    ProjectCount,
    Admin,
}

/// Input parameters for batch project creation.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProjectInput {
    pub freelancer: Address,
    pub amount: i128,
    pub description: String,
    pub github_repo: String,
}

#[contract]
pub struct AgenticPayContract;

#[contractimpl]
impl AgenticPayContract {
    /// Initialize the contract with an admin address
    pub fn initialize(env: Env, admin: Address) {
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::ProjectCount, &0u64);
    }

    /// Create a new project with escrow
    pub fn create_project(
        env: Env,
        client: Address,
        freelancer: Address,
        amount: i128,
        description: String,
        github_repo: String,
    ) -> u64 {
        client.require_auth();

        let mut count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ProjectCount)
            .unwrap_or(0);
        count += 1;

        let project = Project {
            id: count,
            client: client.clone(),
            freelancer: freelancer.clone(),
            amount,
            deposited: 0,
            status: ProjectStatus::Created,
            github_repo,
            description,
            created_at: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Project(count), &project);
        env.storage().instance().set(&DataKey::ProjectCount, &count);

        env.events().publish(
            (symbol_short!("project"), symbol_short!("created")),
            (count, client, freelancer, amount),
        );

        count
    }

    /// Create multiple projects in a single call.
    ///
    /// Optimizes storage writes by reading the project counter once,
    /// writing all projects, then updating the counter once.
    /// Emits a "project/created" event for each project.
    ///
    /// # Arguments
    /// * `client` - Address of the client creating all projects (must authorize)
    /// * `projects` - Vec of ProjectInput structs
    ///
    /// # Returns
    /// Vec of created project IDs
    pub fn batch_create_projects(
        env: Env,
        client: Address,
        projects: Vec<ProjectInput>,
    ) -> Vec<u64> {
        client.require_auth();

        let mut count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ProjectCount)
            .unwrap_or(0);

        let timestamp = env.ledger().timestamp();
        let mut ids = Vec::new(&env);

        for i in 0..projects.len() {
            let input = projects.get(i).expect("Invalid project input");
            count += 1;

            let project = Project {
                id: count,
                client: client.clone(),
                freelancer: input.freelancer.clone(),
                amount: input.amount,
                deposited: 0,
                status: ProjectStatus::Created,
                github_repo: input.github_repo,
                description: input.description,
                created_at: timestamp,
            };

            env.storage()
                .persistent()
                .set(&DataKey::Project(count), &project);

            env.events().publish(
                (symbol_short!("project"), symbol_short!("created")),
                (count, client.clone(), input.freelancer, input.amount),
            );

            ids.push_back(count);
        }

        // Single counter update after all projects are created
        env.storage().instance().set(&DataKey::ProjectCount, &count);

        ids
    }

    /// Fund a project escrow with XLM
    pub fn fund_project(env: Env, project_id: u64, client: Address, amount: i128) {
        client.require_auth();

        let mut project: Project = env
            .storage()
            .persistent()
            .get(&DataKey::Project(project_id))
            .expect("Project not found");

        assert!(project.client == client, "Only client can fund");
        assert!(
            project.status == ProjectStatus::Created,
            "Project must be in Created status"
        );

        project.deposited += amount;
        if project.deposited >= project.amount {
            project.status = ProjectStatus::Funded;
        }

        env.storage()
            .persistent()
            .set(&DataKey::Project(project_id), &project);

        env.events().publish(
            (symbol_short!("project"), symbol_short!("funded")),
            (project_id, amount),
        );
    }

    /// Freelancer submits work with a GitHub repo reference
    pub fn submit_work(env: Env, project_id: u64, freelancer: Address, github_repo: String) {
        freelancer.require_auth();

        let mut project: Project = env
            .storage()
            .persistent()
            .get(&DataKey::Project(project_id))
            .expect("Project not found");

        assert!(
            project.freelancer == freelancer,
            "Only assigned freelancer can submit"
        );
        assert!(
            project.status == ProjectStatus::Funded || project.status == ProjectStatus::InProgress,
            "Project must be funded or in progress"
        );

        project.github_repo = github_repo.clone();
        project.status = ProjectStatus::WorkSubmitted;

        env.storage()
            .persistent()
            .set(&DataKey::Project(project_id), &project);

        env.events().publish(
            (symbol_short!("project"), symbol_short!("work_sub")),
            (project_id, github_repo),
        );
    }

    /// Approve work and release escrow funds to freelancer
    pub fn approve_work(env: Env, project_id: u64, client: Address) {
        client.require_auth();

        let mut project: Project = env
            .storage()
            .persistent()
            .get(&DataKey::Project(project_id))
            .expect("Project not found");

        assert!(project.client == client, "Only client can approve");
        assert!(
            project.status == ProjectStatus::WorkSubmitted
                || project.status == ProjectStatus::Verified,
            "Work must be submitted or verified"
        );

        // TODO: Transfer deposited funds to freelancer via Stellar token transfer

        let amount_released = project.deposited;
        project.status = ProjectStatus::Completed;
        project.deposited = 0;

        env.storage()
            .persistent()
            .set(&DataKey::Project(project_id), &project);

        env.events().publish(
            (symbol_short!("project"), symbol_short!("payment")),
            (project_id, amount_released),
        );
    }

    /// Raise a dispute on a project
    pub fn raise_dispute(env: Env, project_id: u64, caller: Address) {
        caller.require_auth();

        let mut project: Project = env
            .storage()
            .persistent()
            .get(&DataKey::Project(project_id))
            .expect("Project not found");

        assert!(
            caller == project.client || caller == project.freelancer,
            "Only client or freelancer can dispute"
        );

        project.status = ProjectStatus::Disputed;

        env.storage()
            .persistent()
            .set(&DataKey::Project(project_id), &project);

        env.events().publish(
            (symbol_short!("project"), symbol_short!("disputed")),
            (project_id, caller),
        );
    }

    /// Admin resolves a dispute
    pub fn resolve_dispute(env: Env, project_id: u64, admin: Address, release_to_freelancer: bool) {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        assert!(admin == stored_admin, "Only admin can resolve disputes");

        let mut project: Project = env
            .storage()
            .persistent()
            .get(&DataKey::Project(project_id))
            .expect("Project not found");

        assert!(
            project.status == ProjectStatus::Disputed,
            "Project must be disputed"
        );

        if release_to_freelancer {
            // TODO: Transfer funds to freelancer
            project.status = ProjectStatus::Completed;
        } else {
            // TODO: Refund funds to client
            project.status = ProjectStatus::Cancelled;
        }

        project.deposited = 0;
        env.storage()
            .persistent()
            .set(&DataKey::Project(project_id), &project);
    }

    /// Get project details
    pub fn get_project(env: Env, project_id: u64) -> Project {
        env.storage()
            .persistent()
            .get(&DataKey::Project(project_id))
            .expect("Project not found")
    }

    /// Get total project count
    pub fn get_project_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::ProjectCount)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[test]
    fn test_project_creation() {
        let env = Env::default();
        let _admin = Address::generate(&env);
        let client = Address::generate(&env);
        let freelancer = Address::generate(&env);

        let project = Project {
            id: 1,
            client,
            freelancer,
            amount: 1000,
            deposited: 0,
            status: ProjectStatus::Created,
            github_repo: String::from_str(&env, "https://github.com/example/repo"),
            description: String::from_str(&env, "Test project"),
            created_at: env.ledger().timestamp(),
        };

        assert_eq!(project.amount, 1000);
        assert_eq!(project.status, ProjectStatus::Created);
    }

    #[test]
    fn test_batch_create_projects() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, AgenticPayContract);
        let client = AgenticPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let freelancer1 = Address::generate(&env);
        let freelancer2 = Address::generate(&env);
        let freelancer3 = Address::generate(&env);

        client.initialize(&admin);

        let mut inputs = Vec::new(&env);
        inputs.push_back(ProjectInput {
            freelancer: freelancer1.clone(),
            amount: 1000,
            description: String::from_str(&env, "Project 1"),
            github_repo: String::from_str(&env, "https://github.com/test/1"),
        });
        inputs.push_back(ProjectInput {
            freelancer: freelancer2.clone(),
            amount: 2000,
            description: String::from_str(&env, "Project 2"),
            github_repo: String::from_str(&env, "https://github.com/test/2"),
        });
        inputs.push_back(ProjectInput {
            freelancer: freelancer3.clone(),
            amount: 3000,
            description: String::from_str(&env, "Project 3"),
            github_repo: String::from_str(&env, "https://github.com/test/3"),
        });

        let ids = client.batch_create_projects(&user, &inputs);

        // Should return 3 IDs
        assert_eq!(ids.len(), 3);
        assert_eq!(ids.get(0).unwrap(), 1);
        assert_eq!(ids.get(1).unwrap(), 2);
        assert_eq!(ids.get(2).unwrap(), 3);

        // Counter should be updated
        assert_eq!(client.get_project_count(), 3);

        // Verify each project
        let p1 = client.get_project(&1);
        assert_eq!(p1.amount, 1000);
        assert_eq!(p1.freelancer, freelancer1);

        let p2 = client.get_project(&2);
        assert_eq!(p2.amount, 2000);
        assert_eq!(p2.freelancer, freelancer2);

        let p3 = client.get_project(&3);
        assert_eq!(p3.amount, 3000);
        assert_eq!(p3.freelancer, freelancer3);
    }

    #[test]
    fn test_batch_create_empty() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, AgenticPayContract);
        let client = AgenticPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let user = Address::generate(&env);

        client.initialize(&admin);

        let inputs = Vec::new(&env);
        let ids = client.batch_create_projects(&user, &inputs);

        assert_eq!(ids.len(), 0);
        assert_eq!(client.get_project_count(), 0);
    }

    #[test]
    fn test_batch_then_single_create() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, AgenticPayContract);
        let client = AgenticPayContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let freelancer = Address::generate(&env);

        client.initialize(&admin);

        // Batch create 2 projects
        let mut inputs = Vec::new(&env);
        inputs.push_back(ProjectInput {
            freelancer: freelancer.clone(),
            amount: 500,
            description: String::from_str(&env, "Batch 1"),
            github_repo: String::from_str(&env, "https://github.com/b1"),
        });
        inputs.push_back(ProjectInput {
            freelancer: freelancer.clone(),
            amount: 600,
            description: String::from_str(&env, "Batch 2"),
            github_repo: String::from_str(&env, "https://github.com/b2"),
        });
        client.batch_create_projects(&user, &inputs);

        // Then create a single project — ID should be 3
        let id = client.create_project(
            &user,
            &freelancer,
            &700,
            &String::from_str(&env, "Single"),
            &String::from_str(&env, "https://github.com/s1"),
        );

        assert_eq!(id, 3);
        assert_eq!(client.get_project_count(), 3);
    }
}
