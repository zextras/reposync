library(
    identifier: 'jenkins-lib-common@v4.9.1',
    retriever: modernSCM([
        $class: 'GitSCMSource',
        credentialsId: 'jenkins-integration-with-github-account',
        remote: 'git@github.com:zextras/jenkins-lib-common.git',
    ])
)

properties(defaultPipelineProperties())

pipeline {
    agent {
        node {
            label 'rust-v1'
        }
    }
    options {
        buildDiscarder(logRotator(numToKeepStr: '20'))
        disableConcurrentBuilds()
        timeout(time: 1, unit: 'HOURS')
        skipDefaultCheckout()
    }
    stages {
        stage('Checkout') {
            steps {
                checkout scm
                gitMetadata()
            }
        }
        stage('Security Scan') {
            steps { gitleaksStage() }
        }
        stage('Build') {
            steps {
                container('rust') {
                    sh 'cargo build --release'
                }
            }
        }
        stage('Tests') {
            steps {
                container('rust') {
                    sh 'cargo test'
                }
            }
        }
        stage('Docker') {
            when {
                buildingTag()
            }
            steps {
                script {
                    dockerStage(
                        imageName: 'reposync',
                        dockerfile: 'deployment/Dockerfile',
                        registrySpace: 'ci'
                    )
                }
            }
        }
    }
    post {
        always {
            emailext attachLog: true, body: '$DEFAULT_CONTENT', recipientProviders: [requestor()], subject: '$DEFAULT_SUBJECT', to: "${env.GIT_COMMIT_EMAIL}"
        }
    }
}
