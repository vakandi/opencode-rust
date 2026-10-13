use std::collections::HashMap;

pub struct BashArity;

impl BashArity {
    pub fn prefix(tokens: &[String]) -> Vec<String> {
        for len in (1..=tokens.len()).rev() {
            let prefix: String = tokens[..len].join(" ");
            if let Some(&arity) = ARITY.get(prefix.as_str()) {
                return tokens[..arity].to_vec();
            }
        }

        if tokens.is_empty() {
            return vec![];
        }

        tokens[..1].to_vec()
    }
}

lazy_static::lazy_static! {
    static ref ARITY: HashMap<&'static str, usize> = {
        let mut m = HashMap::new();
        // Single token commands
        m.insert("cat", 1);
        m.insert("cd", 1);
        m.insert("chmod", 1);
        m.insert("chown", 1);
        m.insert("cp", 1);
        m.insert("echo", 1);
        m.insert("env", 1);
        m.insert("export", 1);
        m.insert("grep", 1);
        m.insert("kill", 1);
        m.insert("killall", 1);
        m.insert("ln", 1);
        m.insert("ls", 1);
        m.insert("mkdir", 1);
        m.insert("mv", 1);
        m.insert("ps", 1);
        m.insert("pwd", 1);
        m.insert("rm", 1);
        m.insert("rmdir", 1);
        m.insert("sleep", 1);
        m.insert("source", 1);
        m.insert("tail", 1);
        m.insert("touch", 1);
        m.insert("unset", 1);
        m.insert("which", 1);
        // Two token commands
        m.insert("aws", 3);
        m.insert("az", 3);
        m.insert("bazel", 2);
        m.insert("brew", 2);
        m.insert("bun", 2);
        m.insert("bun run", 3);
        m.insert("bun x", 3);
        m.insert("cargo", 2);
        m.insert("cargo add", 3);
        m.insert("cargo run", 3);
        m.insert("cdk", 2);
        m.insert("cf", 2);
        m.insert("cmake", 2);
        m.insert("composer", 2);
        m.insert("consul", 2);
        m.insert("consul kv", 3);
        m.insert("crictl", 2);
        m.insert("deno", 2);
        m.insert("deno task", 3);
        m.insert("doctl", 3);
        m.insert("docker", 2);
        m.insert("docker builder", 3);
        m.insert("docker compose", 3);
        m.insert("docker container", 3);
        m.insert("docker image", 3);
        m.insert("docker network", 3);
        m.insert("docker volume", 3);
        m.insert("eksctl", 2);
        m.insert("eksctl create", 3);
        m.insert("firebase", 2);
        m.insert("flyctl", 2);
        m.insert("gcloud", 3);
        m.insert("gh", 3);
        m.insert("git", 2);
        m.insert("git config", 3);
        m.insert("git remote", 3);
        m.insert("git stash", 3);
        m.insert("go", 2);
        m.insert("gradle", 2);
        m.insert("helm", 2);
        m.insert("heroku", 2);
        m.insert("hugo", 2);
        m.insert("ip", 2);
        m.insert("ip addr", 3);
        m.insert("ip link", 3);
        m.insert("ip netns", 3);
        m.insert("ip route", 3);
        m.insert("kind", 2);
        m.insert("kind create", 3);
        m.insert("kubectl", 2);
        m.insert("kubectl kustomize", 3);
        m.insert("kubectl rollout", 3);
        m.insert("kustomize", 2);
        m.insert("make", 2);
        m.insert("mc", 2);
        m.insert("mc admin", 3);
        m.insert("minikube", 2);
        m.insert("mongosh", 2);
        m.insert("mysql", 2);
        m.insert("mvn", 2);
        m.insert("ng", 2);
        m.insert("npm", 2);
        m.insert("npm exec", 3);
        m.insert("npm init", 3);
        m.insert("npm run", 3);
        m.insert("npm view", 3);
        m.insert("nvm", 2);
        m.insert("nx", 2);
        m.insert("openssl", 2);
        m.insert("openssl req", 3);
        m.insert("openssl x509", 3);
        m.insert("pip", 2);
        m.insert("pipenv", 2);
        m.insert("pnpm", 2);
        m.insert("pnpm dlx", 3);
        m.insert("pnpm exec", 3);
        m.insert("pnpm run", 3);
        m.insert("poetry", 2);
        m.insert("podman", 2);
        m.insert("podman container", 3);
        m.insert("podman image", 3);
        m.insert("psql", 2);
        m.insert("pulumi", 2);
        m.insert("pulumi stack", 3);
        m.insert("pyenv", 2);
        m.insert("python", 2);
        m.insert("rake", 2);
        m.insert("rbenv", 2);
        m.insert("redis-cli", 2);
        m.insert("rustup", 2);
        m.insert("serverless", 2);
        m.insert("sfdx", 3);
        m.insert("skaffold", 2);
        m.insert("sls", 2);
        m.insert("sst", 2);
        m.insert("swift", 2);
        m.insert("systemctl", 2);
        m.insert("terraform", 2);
        m.insert("terraform workspace", 3);
        m.insert("tmux", 2);
        m.insert("turbo", 2);
        m.insert("ufw", 2);
        m.insert("vault", 2);
        m.insert("vault auth", 3);
        m.insert("vault kv", 3);
        m.insert("vercel", 2);
        m.insert("volta", 2);
        m.insert("wp", 2);
        m.insert("yarn", 2);
        m.insert("yarn dlx", 3);
        m.insert("yarn run", 3);
        m
    };
}
