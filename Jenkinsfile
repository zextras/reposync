def buildImage
pipeline {
    agent {
        node {
            label 'rust-agent-latest'
        }
    }
    environment {
        BASE_URL="registry.dev.zextras.com/infra/"
        IMAGE_NAME="reposync"
        IMAGE_VERSION="latest"
        DOCKER_ARGS="--no-cache ."
        BUILD_DOCKER_NAME=BUILD_TAG.replaceAll(" ", "-").replaceAll("%2", "-").toLowerCase()
    }
    options {
        buildDiscarder(logRotator(numToKeepStr: '20'))
    }
    stages {
        stage('Build') {
            steps {
                sh 'cargo build'
            }
        }
        stage('Tests') {
            steps {
                sh 'cargo test'
            }
        }
        stage('Docker') {
            when {
                buildingTag()
            }
            stages {
                stage('Build Release Image') {
                    steps {
                        sh 'cargo build --release'
                        sh 'cp -a target/release/reposync deployment/'
                        dir('deployment') {
                            script {
                                buildImage = docker.build("${BASE_URL}${IMAGE_NAME}:${BUILD_DOCKER_NAME}", env.DOCKER_ARGS)
                            }
                        }
                    }
                }
                stage('Push Image') {
                    steps {
                        dir('deployment') {
                            script {
                                buildImage.push(env.IMAGE_VERSION)
                            }
                        }
                    }
                }
            }
        }
    }
    post {
        always {
            script {
                GIT_COMMIT_EMAIL = sh (
                        script: 'git --no-pager show -s --format=\'%ae\'',
                        returnStdout: true
                ).trim()
            }
            emailext attachLog: true, body: '$DEFAULT_CONTENT', recipientProviders: [requestor()], subject: '$DEFAULT_SUBJECT',  to: "${GIT_COMMIT_EMAIL}"
        }
    }
}
